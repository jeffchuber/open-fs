use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};

use openfs_core::Backend;
use serde_json::{json, Value};

use crate::agent::AgentVm;
use crate::ops::MountId;
use crate::sim::Sim;

/// Build a structured snapshot of the current simulation state.
pub async fn snapshot_json(sim: &Sim) -> Value {
    let mut agents_json = Vec::new();

    for agent in &sim.agents {
        let work_local = snapshot_mount_via_router(agent, MountId::Work).await;
        let indexed_local = snapshot_mount_via_router(agent, MountId::Indexed).await;
        let shared_write_local = snapshot_mount_via_router(agent, MountId::SharedWrite).await;

        let work_raw = snapshot_backend(agent.work_backend.as_ref()).await;
        let indexed_raw = snapshot_backend(agent.indexed_backend.as_ref()).await;
        let shared_read_raw = snapshot_backend(agent.shared_read.as_ref()).await;
        let shared_write_raw = snapshot_backend(agent.shared_write.as_ref()).await;

        let unsynced_local_only = local_only_paths(&indexed_local, &indexed_raw);

        let sync = if let Some(handle) = &agent.write_back_handle {
            let status = handle.status().await;
            let wal = if let Some(wal) = handle.wal() {
                let (pending, processing, failed, wal_unapplied) = wal
                    .outbox_stats()
                    .map(|stats| {
                        (
                            stats.pending,
                            stats.processing,
                            stats.failed,
                            stats.wal_unapplied,
                        )
                    })
                    .unwrap_or((0, 0, 0, 0));

                let mut wal_paths_pending = Vec::new();
                let mut wal_paths_processing = Vec::new();
                let mut wal_paths_failed = Vec::new();
                if let Ok(entries) = wal.outbox_entries() {
                    for entry in entries {
                        match entry.status {
                            openfs_remote::wal::OutboxStatus::Pending => {
                                wal_paths_pending.push(entry.path)
                            }
                            openfs_remote::wal::OutboxStatus::Processing => {
                                wal_paths_processing.push(entry.path)
                            }
                            openfs_remote::wal::OutboxStatus::Failed => {
                                wal_paths_failed.push(entry.path)
                            }
                        }
                    }
                }
                wal_paths_pending.sort();
                wal_paths_pending.dedup();
                wal_paths_processing.sort();
                wal_paths_processing.dedup();
                wal_paths_failed.sort();
                wal_paths_failed.dedup();

                json!({
                    "present": true,
                    "pending": pending,
                    "processing": processing,
                    "failed": failed,
                    "wal_unapplied": wal_unapplied,
                    "paths": {
                        "pending": wal_paths_pending,
                        "processing": wal_paths_processing,
                        "failed": wal_paths_failed,
                    }
                })
            } else {
                json!({
                    "present": false,
                    "pending": 0,
                    "processing": 0,
                    "failed": 0,
                    "wal_unapplied": 0,
                    "paths": {
                        "pending": [],
                        "processing": [],
                        "failed": [],
                    }
                })
            };
            json!({
                "mode": format!("{:?}", status.sync_mode),
                "read_only": status.read_only,
                "cache": {
                    "hits": status.cache.hits,
                    "misses": status.cache.misses,
                    "hit_rate": status.cache.hit_rate(),
                    "entries": status.cache.entries,
                    "size": status.cache.size,
                    "evictions": status.cache.evictions,
                    "expirations": status.cache.expirations,
                },
                "sync": {
                    "synced": status.sync.synced,
                    "pending": status.sync.pending,
                    "failed": status.sync.failed,
                    "retries": status.sync.retries,
                    "last_sync_ago_ms": status
                        .sync
                        .last_sync
                        .map(|instant| instant.elapsed().as_millis() as u64),
                },
                "wal": wal,
            })
        } else {
            Value::Null
        };

        let op_state = sim
            .agent_states
            .get(agent.id)
            .map(agent_state_json)
            .unwrap_or(Value::Null);

        let faults = agent.fault_stats();

        agents_json.push(json!({
            "id": agent.id,
            "fault_stats": {
                "fault_count": faults.fault_count,
                "corruption_count": faults.corruption_count,
            },
            "op_state": op_state,
            "mounts": {
                "work_local": work_local.to_json(),
                "indexed_local": indexed_local.to_json(),
                "shared_write_local": shared_write_local.to_json(),
            },
            "raw_backends": {
                "work": work_raw.to_json(),
                "indexed": indexed_raw.to_json(),
                "shared_read": shared_read_raw.to_json(),
                "shared_write": shared_write_raw.to_json(),
            },
            "sync": sync,
            "unsynced_indexed_local_only": unsynced_local_only,
        }));
    }

    let mut pending_write_back_paths: Vec<String> =
        sim.pending_write_back_paths.iter().cloned().collect();
    pending_write_back_paths.sort();

    let violations: Vec<Value> = sim
        .violations
        .iter()
        .map(|v| {
            json!({
                "step": v.step,
                "agent_id": v.agent_id,
                "invariant": v.invariant,
                "details": v.details,
            })
        })
        .collect();

    let trace: Vec<Value> = sim
        .trace
        .iter()
        .map(|entry| {
            json!({
                "step": entry.step,
                "agent_id": entry.agent_id,
                "op": entry.op,
                "expected": entry.expected,
                "actual": entry.actual,
                "note": entry.note,
            })
        })
        .collect();

    let history: Vec<Value> = sim
        .history
        .iter()
        .map(|frame| {
            let agents: Vec<Value> = frame
                .agents
                .iter()
                .map(|agent| {
                    json!({
                        "agent_id": agent.agent_id,
                        "work_paths": agent.work_paths,
                        "indexed_paths": agent.indexed_paths,
                        "shared_write_paths": agent.shared_write_paths,
                        "remote_indexed_paths": agent.remote_indexed_paths,
                        "remote_indexed_error": agent.remote_indexed_error,
                        "sync": agent.sync.as_ref().map(|sync| {
                            json!({
                                "mode": sync.mode,
                                "pending": sync.pending,
                                "synced": sync.synced,
                                "failed": sync.failed,
                                "retries": sync.retries,
                                "cache_entries": sync.cache_entries,
                                "cache_hits": sync.cache_hits,
                                "cache_misses": sync.cache_misses,
                                "wal_present": sync.wal_present,
                                "wal_pending": sync.wal_pending,
                                "wal_processing": sync.wal_processing,
                                "wal_failed": sync.wal_failed,
                                "wal_unapplied": sync.wal_unapplied,
                                "wal_paths_pending": sync.wal_paths_pending,
                                "wal_paths_processing": sync.wal_paths_processing,
                                "wal_paths_failed": sync.wal_paths_failed,
                            })
                        }),
                    })
                })
                .collect();

            json!({
                "frame": frame.frame,
                "step": frame.step,
                "label": frame.label,
                "violations": frame.violations,
                "pending_write_back_paths": frame.pending_write_back_paths,
                "remote0_paths": frame.remote0_paths,
                "remote0_error": frame.remote0_error,
                "agents": agents,
            })
        })
        .collect();

    let mut oracle_agents = Vec::new();
    for agent_id in 0..sim.agents.len() {
        oracle_agents.push(json!({
            "agent_id": agent_id,
            "work": oracle_map_snapshot(sim.oracle.files_for(agent_id, MountId::Work)),
            "indexed": oracle_map_snapshot(sim.oracle.files_for(agent_id, MountId::Indexed)),
        }));
    }

    let mut last_writers: BTreeMap<String, usize> = BTreeMap::new();
    for (path, writer) in sim.oracle.shared_write_last_writers() {
        last_writers.insert(path.clone(), *writer);
    }

    let remote0 = if let Some(agent) = sim.agents.iter().find(|a| a.id == 2) {
        snapshot_backend(agent.indexed_backend.as_ref())
            .await
            .to_json()
    } else {
        json!({
            "error": "remote0 not configured",
            "files": [],
            "total_bytes": 0,
        })
    };

    json!({
        "meta": {
            "step": sim.step,
            "has_faults": sim.has_faults,
            "agent_count": sim.agents.len(),
            "trace_entries": sim.trace.len(),
            "violations": sim.violations.len(),
            "history_frames": sim.history.len(),
        },
        "pending_write_back_paths": pending_write_back_paths,
        "agents": agents_json,
        "remote0": remote0,
        "oracle": {
            "agents": oracle_agents,
            "shared_read": oracle_map_snapshot(sim.oracle.files_for(0, MountId::SharedRead)),
            "shared_write": oracle_map_snapshot(sim.oracle.shared_write_files()),
            "shared_write_last_writers": last_writers,
        },
        "violations": violations,
        "trace": trace,
        "history": history,
    })
}

/// Render an interactive HTML dashboard from the current simulation state.
pub async fn render_html(sim: &Sim) -> String {
    let snapshot = snapshot_json(sim).await;
    render_html_from_snapshot(&snapshot)
}

/// Write `sim-debug-ui.html` and `sim-debug-data.json` into `output_dir`.
pub async fn write_bundle(
    sim: &Sim,
    output_dir: impl AsRef<Path>,
) -> io::Result<(PathBuf, PathBuf)> {
    let snapshot = snapshot_json(sim).await;
    let output_dir = output_dir.as_ref();
    std::fs::create_dir_all(output_dir)?;

    let html_path = output_dir.join("sim-debug-ui.html");
    let json_path = output_dir.join("sim-debug-data.json");

    let pretty_json = serde_json::to_vec_pretty(&snapshot).map_err(|err| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("failed to serialize sim snapshot: {}", err),
        )
    })?;

    std::fs::write(&json_path, pretty_json)?;
    std::fs::write(&html_path, render_html_from_snapshot(&snapshot))?;

    Ok((html_path, json_path))
}

fn render_html_from_snapshot(snapshot: &Value) -> String {
    let json_payload = serde_json::to_string(snapshot)
        .unwrap_or_else(|_| "{}".to_string())
        .replace("</script>", "<\\/script>");

    let mut html = String::new();
    html.push_str(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>OpenFS Sim Debug UI</title>
  <style>
    :root {
      --bg: #f3efe6;
      --panel: #fff9ef;
      --ink: #1d241e;
      --muted: #5d665f;
      --accent: #1b7f79;
      --accent-2: #d3643b;
      --line: #d8cbb3;
      --warn: #b43d2d;
      --ok: #1d8a4b;
      --shadow: 0 12px 28px rgba(37, 45, 28, 0.12);
    }

    * { box-sizing: border-box; }

    body {
      margin: 0;
      font-family: "Space Grotesk", "Avenir Next", "Segoe UI", sans-serif;
      color: var(--ink);
      background:
        radial-gradient(1200px 600px at 10% -10%, rgba(27, 127, 121, 0.18), transparent),
        radial-gradient(900px 500px at 110% 0%, rgba(211, 100, 59, 0.12), transparent),
        var(--bg);
    }

    main {
      width: min(1360px, 100% - 2rem);
      margin: 1rem auto 3rem;
    }

    .hero {
      background: linear-gradient(120deg, rgba(27, 127, 121, 0.14), rgba(211, 100, 59, 0.12));
      border: 1px solid var(--line);
      border-radius: 16px;
      padding: 1rem 1.2rem;
      box-shadow: var(--shadow);
    }

    h1 {
      margin: 0;
      letter-spacing: 0.02em;
      font-size: clamp(1.2rem, 3vw, 1.8rem);
    }

    .sub {
      margin-top: 0.4rem;
      color: var(--muted);
      font-size: 0.95rem;
    }

    .cards {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
      gap: 0.6rem;
      margin-top: 1rem;
    }

    .card {
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 12px;
      padding: 0.7rem;
      box-shadow: var(--shadow);
    }

    .k {
      color: var(--muted);
      font-size: 0.75rem;
      text-transform: uppercase;
      letter-spacing: 0.07em;
    }

    .v {
      margin-top: 0.2rem;
      font-size: 1.15rem;
      font-weight: 700;
    }

    section {
      margin-top: 1rem;
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 12px;
      padding: 0.8rem;
      box-shadow: var(--shadow);
    }

    h2 {
      margin: 0 0 0.5rem;
      font-size: 1rem;
      letter-spacing: 0.04em;
      text-transform: uppercase;
    }

    .muted { color: var(--muted); }

    .agent-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
      gap: 0.8rem;
    }

    .agent {
      border: 1px solid var(--line);
      border-radius: 10px;
      padding: 0.7rem;
      background: #fffdf7;
    }

    .agent h3 {
      margin: 0;
      font-size: 1rem;
    }

    .pill-row {
      display: flex;
      flex-wrap: wrap;
      gap: 0.35rem;
      margin: 0.5rem 0;
    }

    .pill {
      border: 1px solid var(--line);
      border-radius: 999px;
      padding: 0.12rem 0.55rem;
      background: #fff;
      font-size: 0.78rem;
      white-space: nowrap;
    }

    .pill.warn {
      color: var(--warn);
      border-color: color-mix(in srgb, var(--warn) 35%, white);
    }

    .pill.ok {
      color: var(--ok);
      border-color: color-mix(in srgb, var(--ok) 35%, white);
    }

    details {
      margin-top: 0.5rem;
      border: 1px dashed var(--line);
      border-radius: 8px;
      padding: 0.45rem 0.5rem;
      background: #fff;
    }

    summary {
      cursor: pointer;
      font-size: 0.87rem;
      font-weight: 600;
      color: var(--muted);
    }

    table {
      width: 100%;
      border-collapse: collapse;
      margin-top: 0.5rem;
    }

    th,
    td {
      border-bottom: 1px solid var(--line);
      text-align: left;
      padding: 0.32rem 0.4rem;
      font-size: 0.82rem;
      vertical-align: top;
      font-family: "IBM Plex Mono", "Menlo", monospace;
    }

    th {
      color: var(--muted);
      text-transform: uppercase;
      letter-spacing: 0.06em;
      font-size: 0.7rem;
      font-family: "Space Grotesk", "Avenir Next", sans-serif;
    }

    .controls {
      display: flex;
      gap: 0.55rem;
      flex-wrap: wrap;
      align-items: center;
      margin-bottom: 0.6rem;
    }

    .controls select,
    .controls input {
      border: 1px solid var(--line);
      border-radius: 7px;
      padding: 0.35rem 0.48rem;
      background: #fff;
      font: inherit;
      font-size: 0.85rem;
    }

    .paths {
      margin: 0.25rem 0 0;
      padding-left: 1rem;
      font-family: "IBM Plex Mono", "Menlo", monospace;
      font-size: 0.8rem;
    }

    .warning {
      color: var(--warn);
      font-weight: 600;
    }

    .replay-controls {
      display: grid;
      grid-template-columns: auto auto auto 1fr auto;
      gap: 0.5rem;
      align-items: center;
      margin-bottom: 0.7rem;
    }

    .replay-controls button {
      border: 1px solid var(--line);
      border-radius: 8px;
      background: #fff;
      padding: 0.35rem 0.55rem;
      font: inherit;
      cursor: pointer;
    }

    .replay-controls input[type="range"] {
      width: 100%;
      accent-color: var(--accent);
    }

    .replay-summary {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
      gap: 0.5rem;
      margin-bottom: 0.7rem;
    }

    .replay-agent-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
      gap: 0.7rem;
    }

    .mono-list {
      font-family: "IBM Plex Mono", "Menlo", monospace;
      font-size: 0.78rem;
      margin: 0.3rem 0 0;
      padding-left: 1rem;
      max-height: 180px;
      overflow: auto;
    }

    .file-map {
      margin-top: 0.5rem;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: #fff;
      max-height: 260px;
      overflow: auto;
    }

    .file-row {
      display: grid;
      grid-template-columns: minmax(130px, 1fr) minmax(160px, 220px);
      gap: 0.4rem;
      align-items: center;
      padding: 0.3rem 0.45rem;
      border-bottom: 1px solid #efe5d2;
    }

    .file-row:last-child {
      border-bottom: 0;
    }

    .file-name {
      font-family: "IBM Plex Mono", "Menlo", monospace;
      font-size: 0.76rem;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .badge-row {
      display: flex;
      gap: 0.25rem;
      flex-wrap: wrap;
      justify-content: flex-end;
    }

    .badge {
      font-size: 0.68rem;
      border-radius: 999px;
      padding: 0.08rem 0.38rem;
      border: 1px solid var(--line);
      background: #fdf8ee;
      color: #415048;
      white-space: nowrap;
    }

    .badge.local {
      background: rgba(27, 127, 121, 0.12);
      border-color: rgba(27, 127, 121, 0.42);
      color: #0d5a55;
    }

    .badge.remote {
      background: rgba(65, 109, 228, 0.11);
      border-color: rgba(65, 109, 228, 0.35);
      color: #1f3f99;
    }

    .badge.pending {
      background: rgba(236, 159, 29, 0.2);
      border-color: rgba(204, 126, 0, 0.4);
      color: #824f00;
    }

    .badge.wal {
      background: rgba(153, 96, 202, 0.16);
      border-color: rgba(153, 96, 202, 0.43);
      color: #60308e;
    }

    .badge.failed {
      background: rgba(180, 61, 45, 0.13);
      border-color: rgba(180, 61, 45, 0.45);
      color: #9b2f22;
    }

    .legend {
      display: flex;
      flex-wrap: wrap;
      gap: 0.3rem;
      margin: 0.35rem 0;
    }
  </style>
</head>
<body>
  <main>
    <section class="hero">
      <h1>OpenFS Sim Debug UI</h1>
      <div class="sub">Clients, local state, write-back sync, and operation trace from one simulation run.</div>
      <div class="cards" id="summary-cards"></div>
    </section>

    <section>
      <h2>Replay</h2>
      <div class="replay-controls">
        <button id="step-back" type="button">Back</button>
        <button id="play-toggle" type="button">Play</button>
        <button id="step-forward" type="button">Forward</button>
        <input id="replay-slider" type="range" min="0" max="0" value="0" />
        <label>Speed
          <select id="play-speed">
            <option value="600">1x</option>
            <option value="250">2x</option>
            <option value="120">5x</option>
          </select>
        </label>
      </div>
      <div class="replay-summary" id="replay-summary"></div>
      <div class="replay-agent-grid" id="replay-agents"></div>
    </section>

    <section>
      <h2>Remote 0</h2>
      <div id="remote0-current"></div>
      <div id="remote0-frame"></div>
    </section>

    <section>
      <h2>Clients</h2>
      <div class="agent-grid" id="agents"></div>
    </section>

    <section>
      <h2>Oracle</h2>
      <div id="oracle"></div>
    </section>

    <section>
      <h2>Violations</h2>
      <div id="violations"></div>
    </section>

    <section>
      <h2>Timeline</h2>
      <div class="controls">
        <label>Agent
          <select id="agent-filter">
            <option value="all">All</option>
          </select>
        </label>
        <label>Search
          <input id="op-filter" type="text" placeholder="Write, Flush, NotFound..." />
        </label>
      </div>
      <table>
        <thead>
          <tr>
            <th>Step</th>
            <th>Agent</th>
            <th>Op</th>
            <th>Expected</th>
            <th>Actual</th>
            <th>Note</th>
          </tr>
        </thead>
        <tbody id="timeline"></tbody>
      </table>
    </section>
  </main>

  <script id="sim-data" type="application/json">"#,
    );
    html.push_str(&json_payload);
    html.push_str(
        r#"</script>
  <script>
    const data = JSON.parse(document.getElementById('sim-data').textContent || '{}');
    const meta = data.meta || {};

    const esc = (value) => {
      if (value === null || value === undefined) return '';
      return String(value)
        .replaceAll('&', '&amp;')
        .replaceAll('<', '&lt;')
        .replaceAll('>', '&gt;')
        .replaceAll('"', '&quot;')
        .replaceAll("'", '&#39;');
    };

    const formatNum = (n) => new Intl.NumberFormat().format(Number(n || 0));
    const history = Array.isArray(data.history) ? data.history : [];
    let replayIndex = history.length ? history.length - 1 : 0;
    let playTimer = null;

    function hasRemote0Topology() {
      return Number(meta.agent_count || 0) >= 3;
    }

    function indexedRole(agentId) {
      if (!hasRemote0Topology()) return 'indexed backend: per-client';
      if (agentId === 1) return 'indexed backend: shared Remote0 (write-back)';
      if (agentId === 2) return 'indexed backend: shared Remote0 (write-through)';
      return 'indexed backend: private';
    }

    function backingLabel(agentId) {
      if (!hasRemote0Topology()) return 'private';
      return (agentId === 1 || agentId === 2) ? 'Remote0' : 'private';
    }

    function noSyncMessage(agentId) {
      if (hasRemote0Topology() && agentId === 2) {
        return 'No write-back engine (write-through on shared Remote0 with Client 1).';
      }
      if (hasRemote0Topology() && agentId === 0) {
        return 'No write-back engine (write-through on private indexed backend).';
      }
      return 'No write-back engine; writes go directly to backing store (or cache-only mode).';
    }

    function listToSet(values) {
      return new Set(Array.isArray(values) ? values : []);
    }

    function localOnly(localPaths, remotePaths) {
      const remote = listToSet(remotePaths);
      return (localPaths || []).filter((path) => !remote.has(path));
    }

    function renderMonoList(paths) {
      if (!Array.isArray(paths) || paths.length === 0) {
        return '<div class="muted">none</div>';
      }
      return `<ul class="mono-list">${paths.slice(0, 120).map((path) => `<li>${esc(path)}</li>`).join('')}</ul>`;
    }

    function buildFileStateMap(agent, framePendingPaths) {
      const local = listToSet(agent.indexed_paths || []);
      const remote = listToSet(agent.remote_indexed_paths || []);
      const pending = listToSet(framePendingPaths || []);
      const walPending = listToSet(agent.sync?.wal_paths_pending || []);
      const walProcessing = listToSet(agent.sync?.wal_paths_processing || []);
      const walFailed = listToSet(agent.sync?.wal_paths_failed || []);

      const files = new Set([
        ...local,
        ...remote,
        ...pending,
        ...walPending,
        ...walProcessing,
        ...walFailed,
      ]);

      const rows = [...files].sort().map((path) => {
        const badges = [];
        if (local.has(path)) badges.push(['LOCAL', 'local']);
        if (remote.has(path)) badges.push(['BACKING', 'remote']);
        if (pending.has(path)) badges.push(['PENDING', 'pending']);
        if (walPending.has(path)) badges.push(['WAL-PENDING', 'wal']);
        if (walProcessing.has(path)) badges.push(['WAL-PROCESS', 'wal']);
        if (walFailed.has(path)) badges.push(['WAL-FAILED', 'failed']);

        return { path, badges };
      });

      return rows;
    }

    function renderFileStateRows(rows) {
      if (!rows.length) {
        return '<div class="muted">No indexed file state in this frame.</div>';
      }

      return `
        <div class="file-map">
          ${rows.slice(0, 220).map((row) => `
            <div class="file-row">
              <div class="file-name" title="${esc(row.path)}">${esc(row.path)}</div>
              <div class="badge-row">
                ${row.badges.map(([label, klass]) => `<span class="badge ${klass}">${esc(label)}</span>`).join('')}
              </div>
            </div>
          `).join('')}
        </div>
      `;
    }

    function renderCards() {
      const pending = Array.isArray(data.pending_write_back_paths) ? data.pending_write_back_paths.length : 0;
      const cards = [
        ['Step', formatNum(meta.step || 0)],
        ['Clients', formatNum(meta.agent_count || 0)],
        ['Trace Entries', formatNum(meta.trace_entries || 0)],
        ['History Frames', formatNum(meta.history_frames || 0)],
        ['Violations', formatNum(meta.violations || 0)],
        ['Pending Write-Back Paths', formatNum(pending)],
        ['Indexed Topology', hasRemote0Topology() ? 'C1+C2 share Remote0; C0 private' : 'Per-client indexed'],
        ['Fault Injection', meta.has_faults ? 'Enabled' : 'Disabled'],
      ];

      document.getElementById('summary-cards').innerHTML = cards
        .map(([k, v]) => `<div class="card"><div class="k">${esc(k)}</div><div class="v">${esc(v)}</div></div>`)
        .join('');
    }

    function replayCurrentFrame() {
      if (!history.length) return null;
      return history[Math.max(0, Math.min(replayIndex, history.length - 1))];
    }

    function renderReplay() {
      const slider = document.getElementById('replay-slider');
      slider.max = Math.max(0, history.length - 1);
      slider.value = replayIndex;

      const frame = replayCurrentFrame();
      if (!frame) {
        document.getElementById('replay-summary').innerHTML = '<div class="muted">No replay frames recorded.</div>';
        document.getElementById('replay-agents').innerHTML = '';
        return;
      }

      const label = frame.label ? String(frame.label) : 'step';
      const walPendingTotal = (frame.agents || []).reduce(
        (sum, agent) => sum + Number(agent.sync?.wal_pending || 0),
        0,
      );
      document.getElementById('replay-summary').innerHTML = [
        ['Frame', frame.frame],
        ['Step', frame.step],
        ['Label', label],
        ['Violations So Far', frame.violations],
        ['Pending Write-Back', (frame.pending_write_back_paths || []).length],
        ['Remote0 Files', (frame.remote0_paths || []).length],
        ['WAL Pending', walPendingTotal],
      ].map(([k, v]) => `<div class="card"><div class="k">${esc(k)}</div><div class="v">${esc(v)}</div></div>`).join('');

      const agentsHtml = (frame.agents || []).map((agent) => {
        const sync = agent.sync || null;
        const unsynced = localOnly(agent.indexed_paths || [], agent.remote_indexed_paths || []);
        const fileStateRows = buildFileStateMap(agent, frame.pending_write_back_paths || []);
        const role = indexedRole(agent.agent_id);
        const backing = backingLabel(agent.agent_id);
        const syncPills = sync ? `
          <div class="pill">mode ${esc(sync.mode)}</div>
          <div class="pill ${sync.pending ? 'warn' : 'ok'}">pending ${formatNum(sync.pending)}</div>
          <div class="pill ${sync.failed ? 'warn' : 'ok'}">failed ${formatNum(sync.failed)}</div>
          <div class="pill">synced ${formatNum(sync.synced)}</div>
          <div class="pill">retries ${formatNum(sync.retries)}</div>
          <div class="pill">cache entries ${formatNum(sync.cache_entries)}</div>
          <div class="pill ${sync.wal_pending ? 'warn' : 'ok'}">wal pending ${formatNum(sync.wal_pending || 0)}</div>
          <div class="pill ${sync.wal_processing ? 'warn' : 'ok'}">wal processing ${formatNum(sync.wal_processing || 0)}</div>
          <div class="pill ${sync.wal_failed ? 'warn' : 'ok'}">wal failed ${formatNum(sync.wal_failed || 0)}</div>
        ` : `<div class="muted">${esc(noSyncMessage(agent.agent_id))}</div>`;

        return `
          <div class="agent">
            <h3>Client ${agent.agent_id}</h3>
            <div class="muted">${esc(role)}</div>
            <div class="pill-row">
              <div class="pill">local work ${formatNum((agent.work_paths || []).length)}</div>
              <div class="pill">local indexed ${formatNum((agent.indexed_paths || []).length)}</div>
              <div class="pill">backing indexed (${esc(backing)}) ${formatNum((agent.remote_indexed_paths || []).length)}</div>
              <div class="pill">shared write ${formatNum((agent.shared_write_paths || []).length)}</div>
            </div>
            <div class="pill-row">${syncPills}</div>
            <div class="legend">
              <span class="badge local">LOCAL</span>
              <span class="badge remote">BACKING STORE</span>
              <span class="badge pending">PENDING</span>
              <span class="badge wal">WAL-PENDING/PROCESS</span>
              <span class="badge failed">WAL-FAILED</span>
            </div>
            ${renderFileStateRows(fileStateRows)}
            <details>
              <summary>Unsynced indexed paths (${unsynced.length})</summary>
              ${renderMonoList(unsynced)}
            </details>
            <details>
              <summary>Local indexed paths</summary>
              ${renderMonoList(agent.indexed_paths || [])}
            </details>
            <details>
              <summary>Remote indexed paths</summary>
              ${agent.remote_indexed_error ? `<div class="warning">${esc(agent.remote_indexed_error)}</div>` : renderMonoList(agent.remote_indexed_paths || [])}
            </details>
            <details>
              <summary>Work paths</summary>
              ${renderMonoList(agent.work_paths || [])}
            </details>
          </div>
        `;
      }).join('');

      document.getElementById('replay-agents').innerHTML = agentsHtml || '<div class="muted">No agent frames.</div>';
    }

    function renderRemote0Current() {
      const remote0 = data.remote0 || null;
      if (!remote0 || remote0.error) {
        const err = remote0 && remote0.error ? remote0.error : 'remote0 unavailable';
        document.getElementById('remote0-current').innerHTML = `<div class="warning">${esc(err)}</div>`;
        return;
      }

      document.getElementById('remote0-current').innerHTML = `
        <div class="muted">Final Remote0 backing state</div>
        ${renderFileSnapshot(remote0)}
      `;
    }

    function renderRemote0Frame() {
      const frame = replayCurrentFrame();
      if (!frame) {
        document.getElementById('remote0-frame').innerHTML = '';
        return;
      }

      if (frame.remote0_error) {
        document.getElementById('remote0-frame').innerHTML = `<div class="warning">${esc(frame.remote0_error)}</div>`;
        return;
      }

      document.getElementById('remote0-frame').innerHTML = `
        <div class="muted">Replay frame Remote0 paths (${formatNum((frame.remote0_paths || []).length)})</div>
        ${renderMonoList(frame.remote0_paths || [])}
      `;
    }

    function renderFileSnapshot(snapshot) {
      if (!snapshot || snapshot.error) {
        const err = snapshot && snapshot.error ? snapshot.error : 'unavailable';
        return `<div class="muted">Snapshot error: ${esc(err)}</div>`;
      }

      const files = Array.isArray(snapshot.files) ? snapshot.files : [];
      if (!files.length) {
        return '<div class="muted">No files</div>';
      }

      const maxRows = 160;
      const rows = files.slice(0, maxRows).map((file) => {
        if (file.read_error) {
          return `<tr><td>${esc(file.path)}</td><td class="warning">read error</td><td>${esc(file.read_error)}</td></tr>`;
        }
        return `<tr><td>${esc(file.path)}</td><td>${formatNum(file.bytes)}</td><td>${esc(file.preview)}</td></tr>`;
      }).join('');

      const more = files.length > maxRows
        ? `<div class="muted">Showing ${maxRows} / ${files.length} files</div>`
        : '';

      return `
        <div class="muted">files=${files.length} bytes=${formatNum(snapshot.total_bytes || 0)}</div>
        ${more}
        <table>
          <thead><tr><th>Path</th><th>Bytes</th><th>Preview</th></tr></thead>
          <tbody>${rows}</tbody>
        </table>
      `;
    }

    function renderPathList(paths) {
      if (!Array.isArray(paths) || paths.length === 0) {
        return '<div class="muted">none</div>';
      }
      return `<ul class="paths">${paths.map((path) => `<li>${esc(path)}</li>`).join('')}</ul>`;
    }

    function renderAgents() {
      const agents = Array.isArray(data.agents) ? data.agents : [];
      document.getElementById('agent-filter').innerHTML =
        '<option value="all">All</option>' + agents.map((agent) => `<option value="${agent.id}">Agent ${agent.id}</option>`).join('');

      const html = agents.map((agent) => {
        const faults = agent.fault_stats || {};
        const sync = agent.sync;
        const state = agent.op_state || {};
        const known = state.known_files || {};
        const role = indexedRole(agent.id);
        const backing = backingLabel(agent.id);

        const syncPills = sync
          ? `
            <div class="pill">mode ${esc(sync.mode)}</div>
            <div class="pill">cache hits ${formatNum(sync.cache?.hits || 0)}</div>
            <div class="pill">cache misses ${formatNum(sync.cache?.misses || 0)}</div>
            <div class="pill ${sync.sync?.pending ? 'warn' : 'ok'}">pending ${formatNum(sync.sync?.pending || 0)}</div>
            <div class="pill ${sync.sync?.failed ? 'warn' : 'ok'}">failed ${formatNum(sync.sync?.failed || 0)}</div>
            <div class="pill">synced ${formatNum(sync.sync?.synced || 0)}</div>
            <div class="pill">retries ${formatNum(sync.sync?.retries || 0)}</div>
            <div class="pill ${sync.wal?.pending ? 'warn' : 'ok'}">wal pending ${formatNum(sync.wal?.pending || 0)}</div>
            <div class="pill ${sync.wal?.processing ? 'warn' : 'ok'}">wal processing ${formatNum(sync.wal?.processing || 0)}</div>
            <div class="pill ${sync.wal?.failed ? 'warn' : 'ok'}">wal failed ${formatNum(sync.wal?.failed || 0)}</div>
          `
          : `<div class="muted">${esc(noSyncMessage(agent.id))}</div>`;

        return `
          <div class="agent">
            <h3>Client ${agent.id}</h3>
            <div class="muted">${esc(role)}</div>
            <div class="pill-row">
              <div class="pill">faults ${formatNum(faults.fault_count || 0)}</div>
              <div class="pill">corruptions ${formatNum(faults.corruption_count || 0)}</div>
              <div class="pill">known indexed ${formatNum((known.indexed || []).length)}</div>
              <div class="pill">known work ${formatNum((known.work || []).length)}</div>
              <div class="pill">indexed backing ${esc(backing)}</div>
            </div>
            <div class="pill-row">${syncPills}</div>
            <div><strong>Local-only indexed paths (not in backing raw backend):</strong></div>
            ${renderPathList(agent.unsynced_indexed_local_only || [])}
            <details>
              <summary>WAL Paths</summary>
              ${sync?.wal?.present ? `
                <div class="legend">
                  <span class="badge wal">pending ${formatNum(sync.wal.pending || 0)}</span>
                  <span class="badge wal">processing ${formatNum(sync.wal.processing || 0)}</span>
                  <span class="badge failed">failed ${formatNum(sync.wal.failed || 0)}</span>
                  <span class="badge">wal unapplied ${formatNum(sync.wal.wal_unapplied || 0)}</span>
                </div>
                <div><strong>Pending</strong></div>
                ${renderMonoList(sync.wal.paths?.pending || [])}
                <div><strong>Processing</strong></div>
                ${renderMonoList(sync.wal.paths?.processing || [])}
                <div><strong>Failed</strong></div>
                ${renderMonoList(sync.wal.paths?.failed || [])}
              ` : '<div class="muted">WAL not enabled for this client.</div>'}
            </details>

            <details>
              <summary>Indexed Local (router view)</summary>
              ${renderFileSnapshot(agent.mounts?.indexed_local)}
            </details>
            <details>
              <summary>Indexed Backing Store (raw backend)</summary>
              ${renderFileSnapshot(agent.raw_backends?.indexed)}
            </details>
            <details>
              <summary>Work Local (router view)</summary>
              ${renderFileSnapshot(agent.mounts?.work_local)}
            </details>
            <details>
              <summary>Shared Write Local (router view)</summary>
              ${renderFileSnapshot(agent.mounts?.shared_write_local)}
            </details>
          </div>
        `;
      }).join('');

      document.getElementById('agents').innerHTML = html || '<div class="muted">No agents.</div>';
    }

    function renderOracle() {
      const oracle = data.oracle || {};
      const rows = [];
      (oracle.agents || []).forEach((agent) => {
        rows.push(`<tr><td>agent ${agent.agent_id} work</td><td>${formatNum((agent.work || []).length)}</td></tr>`);
        rows.push(`<tr><td>agent ${agent.agent_id} indexed</td><td>${formatNum((agent.indexed || []).length)}</td></tr>`);
      });
      rows.push(`<tr><td>shared read</td><td>${formatNum((oracle.shared_read || []).length)}</td></tr>`);
      rows.push(`<tr><td>shared write</td><td>${formatNum((oracle.shared_write || []).length)}</td></tr>`);

      document.getElementById('oracle').innerHTML = `
        <table>
          <thead><tr><th>State</th><th>Files</th></tr></thead>
          <tbody>${rows.join('')}</tbody>
        </table>
      `;
    }

    function renderViolations() {
      const violations = Array.isArray(data.violations) ? data.violations : [];
      if (!violations.length) {
        document.getElementById('violations').innerHTML = '<div class="pill ok">No violations</div>';
        return;
      }

      const rows = violations.slice(0, 200).map((violation) => `
        <tr>
          <td>${esc(violation.step)}</td>
          <td>${esc(violation.agent_id)}</td>
          <td>${esc(violation.invariant)}</td>
          <td>${esc(violation.details)}</td>
        </tr>
      `).join('');

      const more = violations.length > 200
        ? `<div class="muted">Showing first 200 / ${violations.length} violations</div>`
        : '';

      document.getElementById('violations').innerHTML = `
        ${more}
        <table>
          <thead><tr><th>Step</th><th>Agent</th><th>Invariant</th><th>Details</th></tr></thead>
          <tbody>${rows}</tbody>
        </table>
      `;
    }

    const timeline = Array.isArray(data.trace) ? data.trace : [];

    function renderTimeline() {
      const agent = document.getElementById('agent-filter').value;
      const text = (document.getElementById('op-filter').value || '').trim().toLowerCase();
      const frame = replayCurrentFrame();
      const selectedStep = frame ? Number(frame.step || 0) : Number.MAX_SAFE_INTEGER;

      const rows = timeline
        .filter((entry) => agent === 'all' || String(entry.agent_id) === agent)
        .filter((entry) => Number(entry.step) < selectedStep)
        .filter((entry) => {
          if (!text) return true;
          const hay = `${entry.op} ${entry.expected} ${entry.actual} ${entry.note || ''}`.toLowerCase();
          return hay.includes(text);
        })
        .slice(0, 600)
        .map((entry) => `
          <tr>
            <td>${esc(entry.step)}</td>
            <td>${esc(entry.agent_id)}</td>
            <td>${esc(entry.op)}</td>
            <td>${esc(entry.expected)}</td>
            <td>${esc(entry.actual)}</td>
            <td>${esc(entry.note || '')}</td>
          </tr>
        `)
        .join('');

      document.getElementById('timeline').innerHTML = rows || '<tr><td colspan="6" class="muted">No trace rows</td></tr>';
    }

    function stopPlayback() {
      if (playTimer) {
        clearInterval(playTimer);
      }
      playTimer = null;
      document.getElementById('play-toggle').textContent = 'Play';
    }

    function startPlayback() {
      stopPlayback();
      const speed = Number(document.getElementById('play-speed').value || 600);
      const timer = setInterval(() => {
        if (!history.length) {
          stopPlayback();
          return;
        }
        if (replayIndex >= history.length - 1) {
          stopPlayback();
          return;
        }
        replayIndex += 1;
        renderReplay();
        renderRemote0Frame();
        renderTimeline();
      }, speed);
      playTimer = timer;
      document.getElementById('play-toggle').textContent = 'Pause';
    }

    function isPlaying() {
      return Boolean(playTimer);
    }

    function rerenderAfterSeek() {
      renderReplay();
      renderRemote0Frame();
      renderTimeline();
    }

    document.getElementById('agent-filter').addEventListener('change', renderTimeline);
    document.getElementById('op-filter').addEventListener('input', renderTimeline);
    document.getElementById('replay-slider').addEventListener('input', (event) => {
      replayIndex = Number(event.target.value || 0);
      rerenderAfterSeek();
    });
    document.getElementById('step-back').addEventListener('click', () => {
      replayIndex = Math.max(0, replayIndex - 1);
      rerenderAfterSeek();
    });
    document.getElementById('step-forward').addEventListener('click', () => {
      replayIndex = Math.min(Math.max(0, history.length - 1), replayIndex + 1);
      rerenderAfterSeek();
    });
    document.getElementById('play-toggle').addEventListener('click', () => {
      if (isPlaying()) {
        stopPlayback();
      } else {
        startPlayback();
      }
    });
    document.getElementById('play-speed').addEventListener('change', () => {
      if (isPlaying()) {
        startPlayback();
      }
    });

    renderCards();
    renderAgents();
    renderRemote0Current();
    renderOracle();
    renderViolations();
    renderReplay();
    renderRemote0Frame();
    renderTimeline();
  </script>
</body>
</html>
"#,
    );

    html
}

#[derive(Debug, Clone, Default)]
struct FileEntrySnapshot {
    path: String,
    bytes: usize,
    preview: String,
    read_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct BackendSnapshot {
    root: String,
    relative_root: String,
    files: Vec<FileEntrySnapshot>,
    total_bytes: usize,
    error: Option<String>,
}

impl BackendSnapshot {
    fn to_json(&self) -> Value {
        let files: Vec<Value> = self
            .files
            .iter()
            .map(|file| {
                json!({
                    "path": file.path,
                    "bytes": file.bytes,
                    "preview": file.preview,
                    "read_error": file.read_error,
                })
            })
            .collect();

        json!({
            "root": self.root,
            "relative_root": self.relative_root,
            "files": files,
            "total_bytes": self.total_bytes,
            "error": self.error,
        })
    }

    fn file_paths(&self) -> BTreeSet<String> {
        self.files
            .iter()
            .filter(|file| file.read_error.is_none())
            .map(|file| file.path.clone())
            .collect()
    }
}

async fn snapshot_mount_via_router(agent: &AgentVm, mount: MountId) -> BackendSnapshot {
    let root = mount.prefix(agent.id).to_string();

    match agent.router.resolve(&root) {
        Ok((backend, relative, _)) => {
            let mut snapshot = snapshot_backend_root(backend, &relative).await;
            snapshot.root = root;
            snapshot.relative_root = normalize_rel(&relative);
            snapshot
        }
        Err(err) => BackendSnapshot {
            root,
            error: Some(err.to_string()),
            ..Default::default()
        },
    }
}

async fn snapshot_backend(backend: &dyn Backend) -> BackendSnapshot {
    snapshot_backend_root(backend, "").await
}

async fn snapshot_backend_root(backend: &dyn Backend, root: &str) -> BackendSnapshot {
    let root_norm = normalize_rel(root);

    let paths = match collect_file_paths(backend, &root_norm).await {
        Ok(paths) => paths,
        Err(err) => {
            return BackendSnapshot {
                relative_root: root_norm,
                error: Some(err),
                ..Default::default()
            }
        }
    };

    let mut files = Vec::new();
    let mut total_bytes = 0usize;

    for full_path in paths {
        let display_path = strip_root(&full_path, &root_norm);
        match backend.read(&full_path).await {
            Ok(bytes) => {
                total_bytes += bytes.len();
                files.push(FileEntrySnapshot {
                    path: display_path,
                    bytes: bytes.len(),
                    preview: preview_bytes(&bytes),
                    read_error: None,
                });
            }
            Err(err) => {
                files.push(FileEntrySnapshot {
                    path: display_path,
                    bytes: 0,
                    preview: String::new(),
                    read_error: Some(err.to_string()),
                });
            }
        }
    }

    BackendSnapshot {
        relative_root: root_norm,
        files,
        total_bytes,
        error: None,
        root: String::new(),
    }
}

async fn collect_file_paths(backend: &dyn Backend, root: &str) -> Result<Vec<String>, String> {
    let mut files = BTreeSet::new();
    let mut stack = vec![root.to_string()];
    let mut seen_dirs = BTreeSet::new();

    while let Some(dir) = stack.pop() {
        if !seen_dirs.insert(dir.clone()) {
            continue;
        }

        let entries = backend.list(&dir).await.map_err(|err| err.to_string())?;
        let mut dirs = Vec::new();
        let mut leafs = Vec::new();

        for entry in entries {
            let normalized = normalize_rel(&entry.path);
            if normalized.is_empty() {
                continue;
            }
            if !path_is_under_root(&normalized, root) {
                continue;
            }
            if entry.is_dir {
                dirs.push(normalized);
            } else {
                leafs.push(normalized);
            }
        }

        dirs.sort();
        leafs.sort();

        for dir_path in dirs.into_iter().rev() {
            stack.push(dir_path);
        }

        for file_path in leafs {
            files.insert(file_path);
        }
    }

    Ok(files.into_iter().collect())
}

fn path_is_under_root(path: &str, root: &str) -> bool {
    if root.is_empty() {
        return true;
    }
    path == root || path.starts_with(&format!("{}/", root))
}

fn strip_root(path: &str, root: &str) -> String {
    if root.is_empty() {
        return path.to_string();
    }

    if path == root {
        return String::new();
    }

    if let Some(stripped) = path.strip_prefix(&format!("{}/", root)) {
        return stripped.to_string();
    }

    path.to_string()
}

fn normalize_rel(path: &str) -> String {
    path.trim_matches('/').to_string()
}

fn preview_bytes(bytes: &[u8]) -> String {
    const MAX_PREVIEW_BYTES: usize = 80;

    if bytes.is_empty() {
        return "<empty>".to_string();
    }

    let preview_len = bytes.len().min(MAX_PREVIEW_BYTES);
    let raw = String::from_utf8_lossy(&bytes[..preview_len]);
    let cleaned = raw
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");

    if bytes.len() > MAX_PREVIEW_BYTES {
        format!("{}…", cleaned)
    } else {
        cleaned
    }
}

fn oracle_map_snapshot(map: &HashMap<String, Vec<u8>>) -> Vec<Value> {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();

    keys.into_iter()
        .map(|path| {
            let content = map.get(path).expect("key from map.keys() must exist");
            json!({
                "path": path,
                "bytes": content.len(),
                "preview": preview_bytes(content),
            })
        })
        .collect()
}

fn agent_state_json(state: &crate::ops::AgentOpState) -> Value {
    let mut work = state.known_for(MountId::Work).to_vec();
    let mut indexed = state.known_for(MountId::Indexed).to_vec();
    let mut shared_read = state.known_for(MountId::SharedRead).to_vec();
    let mut shared_write = state.known_for(MountId::SharedWrite).to_vec();
    let mut indexed_files = state.indexed_files.clone();

    work.sort();
    indexed.sort();
    shared_read.sort();
    shared_write.sort();
    indexed_files.sort();

    json!({
        "file_counter": state.file_counter,
        "indexed_files": indexed_files,
        "known_files": {
            "work": work,
            "indexed": indexed,
            "shared_read": shared_read,
            "shared_write": shared_write,
        }
    })
}

fn local_only_paths(local: &BackendSnapshot, remote: &BackendSnapshot) -> Vec<String> {
    let remote_paths = remote.file_paths();
    let mut local_only: Vec<String> = local
        .file_paths()
        .into_iter()
        .filter(|path| !remote_paths.contains(path))
        .collect();
    local_only.sort();
    local_only
}
