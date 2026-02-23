use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use openfs_core::{Backend, BackendError, CacheConfig};
use openfs_remote::{CachedBackend, MemoryBackend};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::{json, Value};
use tokio::task::JoinSet;

use crate::backend_wrapper::DynBackend;

const SKILL_TOPICS: [&str; 4] = [
    "rust_refactoring",
    "test_driven_fixes",
    "api_contracts",
    "async_debugging",
];

/// A single coding-agent action captured in the trace.
#[derive(Debug, Clone)]
pub struct CodingActionTrace {
    pub round: usize,
    pub agent_id: usize,
    pub action: String,
    pub path: String,
    pub bytes: usize,
}

#[derive(Debug, Clone)]
struct AgentRoundPlan {
    round: usize,
    agent_id: usize,
    code_path: String,
    code_expected_token: Option<String>,
    code_delta: String,
    skill_path: String,
    skill_expected_token: Option<String>,
    skill_entry: String,
    memory_path: String,
    memory_entry: String,
}

#[derive(Debug, Clone)]
struct CodingFrameFile {
    path: String,
    bytes: usize,
    contributors: Vec<usize>,
    last_agent: Option<usize>,
    edits: usize,
}

#[derive(Debug, Clone)]
struct CodingFrameAgent {
    agent_id: usize,
    memory_entries: usize,
    memory_bytes: usize,
    last_note: String,
    cumulative_actions: usize,
}

#[derive(Debug, Clone)]
struct CodingSimFrame {
    frame: usize,
    round: usize,
    label: String,
    repo_files: Vec<CodingFrameFile>,
    repo_total_bytes: usize,
    skill_files: Vec<CodingFrameFile>,
    skill_total_bytes: usize,
    agents: Vec<CodingFrameAgent>,
    round_actions: Vec<CodingActionTrace>,
}

#[derive(Debug, Clone, Copy)]
enum EditCounterKind {
    Code,
    Bullets,
}

/// Workload profile for the coding-agent simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodingSimProfile {
    /// Mixed writes with mostly independent edits.
    Balanced,
    /// Intentional hot-spot writes to demonstrate CAS conflicts and retries.
    CasDemo,
}

impl CodingSimProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            CodingSimProfile::Balanced => "balanced",
            CodingSimProfile::CasDemo => "cas-demo",
        }
    }
}

/// Shared/isolated mounts for one simulated coding agent.
#[derive(Clone)]
pub struct CodingAgentVm {
    pub id: usize,
    pub repo_mount: Arc<CachedBackend<DynBackend>>,
    pub skills_mount: Arc<CachedBackend<DynBackend>>,
    pub memory_mount: Arc<CachedBackend<DynBackend>>,
    pub memory_backend: Arc<MemoryBackend>,
}

/// Summary metrics for one simulation run.
#[derive(Debug, Clone)]
pub struct CodingSimSummary {
    pub seed: u64,
    pub profile: CodingSimProfile,
    pub rounds: usize,
    pub agent_count: usize,
    pub trace_entries: usize,
    pub error_count: usize,
    pub cas_conflicts: usize,
    pub cas_retried_writes: usize,
    pub cas_max_retries: usize,
    pub code_file_count: usize,
    pub shared_skill_file_count: usize,
    pub agents_with_memory: usize,
    pub total_memory_entries: usize,
    pub code_contributors: usize,
    pub skill_contributors: usize,
}

/// Multi-agent coding simulation with shared code and skills, plus private memory.
pub struct CodingAgentSim {
    pub seed: u64,
    pub profile: CodingSimProfile,
    pub agents: Vec<CodingAgentVm>,
    pub repo_backend: Arc<MemoryBackend>,
    pub skills_backend: Arc<MemoryBackend>,
    pub rng: ChaCha8Rng,
    pub round: usize,
    pub trace: Vec<CodingActionTrace>,
    pub errors: Vec<String>,
    code_targets: Vec<String>,
    skill_topics: Vec<String>,
    history: Vec<CodingSimFrame>,
    action_counts: Vec<usize>,
}

impl CodingAgentSim {
    /// Build a new coding-agent simulation.
    pub async fn new(seed: u64, agent_count: usize) -> Self {
        Self::new_with_profile(seed, agent_count, CodingSimProfile::Balanced).await
    }

    /// Build a new coding-agent simulation with a specific workload profile.
    pub async fn new_with_profile(
        seed: u64,
        agent_count: usize,
        profile: CodingSimProfile,
    ) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let agent_count = agent_count.max(2);

        let repo_backend = Arc::new(MemoryBackend::new());
        let skills_backend = Arc::new(MemoryBackend::new());

        let mut code_targets: Vec<String> = vec![
            "src/lib.rs".to_string(),
            "src/router.rs".to_string(),
            "src/skills.rs".to_string(),
            "tests/integration_smoke.rs".to_string(),
        ];
        for path in &code_targets {
            let seed_body = format!(
                "// seeded file {}\npub fn seeded_{}() -> &'static str {{ \"seed\" }}\n",
                path,
                sanitize_ident(path)
            );
            repo_backend
                .write(path, seed_body.as_bytes())
                .await
                .expect("seed code file");
        }

        for id in 0..agent_count {
            let path = format!("src/agents/agent_{}.rs", id);
            let seed_body = format!(
                "// owned by agent {}\npub fn agent_{}_bootstrap() -> &'static str {{ \"boot\" }}\n",
                id, id
            );
            repo_backend
                .write(&path, seed_body.as_bytes())
                .await
                .expect("seed per-agent code file");
            code_targets.push(path);
        }

        let skill_topics: Vec<String> = SKILL_TOPICS.iter().map(ToString::to_string).collect();
        for topic in &skill_topics {
            let path = format!("skills/{}.md", topic);
            let header = format!("# {}\n\n## Shared notes\n", topic.replace('_', " "));
            skills_backend
                .write(&path, header.as_bytes())
                .await
                .expect("seed shared skill file");
        }

        let mut agents = Vec::with_capacity(agent_count);
        for id in 0..agent_count {
            let repo_dyn: Arc<dyn Backend> = repo_backend.clone();
            let skills_dyn: Arc<dyn Backend> = skills_backend.clone();
            let memory_backend = Arc::new(MemoryBackend::new());
            let memory_dyn: Arc<dyn Backend> = memory_backend.clone();

            memory_backend
                .write(
                    "memory/notes.md",
                    format!("# Agent {} memory log\n", id).as_bytes(),
                )
                .await
                .expect("seed memory file");

            let repo_mount = Arc::new(CachedBackend::write_through(
                DynBackend(repo_dyn),
                CacheConfig::default(),
            ));
            let skills_mount = Arc::new(CachedBackend::write_through(
                DynBackend(skills_dyn),
                CacheConfig::default(),
            ));
            let memory_mount = Arc::new(CachedBackend::write_through(
                DynBackend(memory_dyn),
                CacheConfig::default(),
            ));

            agents.push(CodingAgentVm {
                id,
                repo_mount,
                skills_mount,
                memory_mount,
                memory_backend,
            });
        }

        // Stir deterministic entropy so low seeds do not align too predictably.
        let _: u32 = rng.gen();

        let mut sim = CodingAgentSim {
            seed,
            profile,
            agents,
            repo_backend,
            skills_backend,
            rng,
            round: 0,
            trace: Vec::new(),
            errors: Vec::new(),
            code_targets,
            skill_topics,
            history: Vec::new(),
            action_counts: vec![0; agent_count],
        };

        if let Err(err) = sim
            .capture_history_frame("initial".to_string(), 0, Vec::new())
            .await
        {
            sim.errors
                .push(format!("failed to capture initial history frame: {}", err));
        }

        sim
    }

    /// Run multiple rounds. Each round executes one coding cycle per agent concurrently.
    pub async fn run_rounds(&mut self, rounds: usize) {
        for _ in 0..rounds {
            let round = self.round;
            let mut join_set = JoinSet::new();
            let mut round_actions = Vec::with_capacity(self.agents.len() * 3);
            let mut round_plans = Vec::with_capacity(self.agents.len());

            for agent_id in 0..self.agents.len() {
                round_plans.push(self.make_plan(agent_id, round));
            }

            if let Err(err) = self.seed_shared_cas_tokens(&mut round_plans).await {
                self.errors.push(format!(
                    "failed to seed round {} CAS tokens: {}",
                    round, err
                ));
            }

            for (agent_id, plan) in round_plans.into_iter().enumerate() {
                let agent = self.agents[agent_id].clone();
                join_set.spawn(async move { execute_agent_plan(agent, plan).await });
            }

            while let Some(joined) = join_set.join_next().await {
                match joined {
                    Ok(Ok(mut actions)) => round_actions.append(&mut actions),
                    Ok(Err(err)) => self.errors.push(err),
                    Err(err) => self.errors.push(format!("join error: {}", err)),
                }
            }

            round_actions.sort_by(|a, b| {
                (a.agent_id, action_rank(&a.action), &a.path).cmp(&(
                    b.agent_id,
                    action_rank(&b.action),
                    &b.path,
                ))
            });

            for action in &round_actions {
                if let Some(total) = self.action_counts.get_mut(action.agent_id) {
                    *total += 1;
                }
            }
            self.trace.extend(round_actions.iter().cloned());

            if let Err(err) = self
                .capture_history_frame(format!("round {}", round), round, round_actions)
                .await
            {
                self.errors.push(format!(
                    "failed to capture history frame for round {}: {}",
                    round, err
                ));
            }

            self.round += 1;
        }
    }

    async fn seed_shared_cas_tokens(&self, plans: &mut [AgentRoundPlan]) -> Result<(), String> {
        let mut code_groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut skill_groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();

        for (index, plan) in plans.iter().enumerate() {
            code_groups
                .entry(plan.code_path.clone())
                .or_default()
                .push(index);
            skill_groups
                .entry(plan.skill_path.clone())
                .or_default()
                .push(index);
        }

        for (path, indices) in code_groups {
            if indices.len() < 2 {
                continue;
            }
            let token = read_backend_cas_token(self.repo_backend.as_ref(), &path).await?;
            if token.is_none() {
                continue;
            }
            for idx in indices {
                plans[idx].code_expected_token = token.clone();
            }
        }

        for (path, indices) in skill_groups {
            if indices.len() < 2 {
                continue;
            }
            let token = read_backend_cas_token(self.skills_backend.as_ref(), &path).await?;
            if token.is_none() {
                continue;
            }
            for idx in indices {
                plans[idx].skill_expected_token = token.clone();
            }
        }

        Ok(())
    }

    async fn capture_history_frame(
        &mut self,
        label: String,
        round: usize,
        round_actions: Vec<CodingActionTrace>,
    ) -> Result<(), String> {
        let repo_files_raw = collect_backend_files(self.repo_backend.as_ref()).await?;
        let skill_files_raw = collect_backend_files(self.skills_backend.as_ref()).await?;

        let (repo_files, repo_total_bytes) =
            snapshot_frame_files(&repo_files_raw, EditCounterKind::Code);
        let (skill_files, skill_total_bytes) =
            snapshot_frame_files(&skill_files_raw, EditCounterKind::Bullets);

        let mut agents = Vec::with_capacity(self.agents.len());
        for agent in &self.agents {
            let memory_files = collect_backend_files(agent.memory_backend.as_ref()).await?;
            let notes = memory_files
                .get("memory/notes.md")
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let cumulative_actions = self.action_counts.get(agent.id).copied().unwrap_or(0);
            agents.push(CodingFrameAgent {
                agent_id: agent.id,
                memory_entries: count_memory_entries(notes),
                memory_bytes: notes.len(),
                last_note: last_round_note(notes),
                cumulative_actions,
            });
        }

        self.history.push(CodingSimFrame {
            frame: self.history.len(),
            round,
            label,
            repo_files,
            repo_total_bytes,
            skill_files,
            skill_total_bytes,
            agents,
            round_actions,
        });
        Ok(())
    }

    /// Compute aggregate metrics from the current state.
    pub async fn summary(&self) -> Result<CodingSimSummary, String> {
        let repo_files = collect_backend_files(self.repo_backend.as_ref()).await?;
        let skill_files = collect_backend_files(self.skills_backend.as_ref()).await?;
        let cas_metrics = cas_metrics_from_actions(&self.trace);

        let mut agents_with_memory = 0usize;
        let mut total_memory_entries = 0usize;
        for agent in &self.agents {
            let memory_files = collect_backend_files(agent.memory_backend.as_ref()).await?;
            if let Some(content) = memory_files.get("memory/notes.md") {
                agents_with_memory += 1;
                total_memory_entries += count_memory_entries(content);
            }
        }

        let code_contributors = contributor_ids_in_files(&repo_files).len();
        let skill_contributors = contributor_ids_in_files(&skill_files).len();

        Ok(CodingSimSummary {
            seed: self.seed,
            profile: self.profile,
            rounds: self.round,
            agent_count: self.agents.len(),
            trace_entries: self.trace.len(),
            error_count: self.errors.len(),
            cas_conflicts: cas_metrics.conflicts,
            cas_retried_writes: cas_metrics.retried_writes,
            cas_max_retries: cas_metrics.max_retries,
            code_file_count: repo_files.len(),
            shared_skill_file_count: skill_files.len(),
            agents_with_memory,
            total_memory_entries,
            code_contributors,
            skill_contributors,
        })
    }

    /// Build a detailed JSON report of the simulation state.
    pub async fn report_json(&self) -> Result<Value, String> {
        let summary = self.summary().await?;
        let repo_files = collect_backend_files(self.repo_backend.as_ref()).await?;
        let skill_files = collect_backend_files(self.skills_backend.as_ref()).await?;

        let mut agents_json = Vec::with_capacity(self.agents.len());
        for agent in &self.agents {
            let memory_files = collect_backend_files(agent.memory_backend.as_ref()).await?;
            let action_count = self.trace.iter().filter(|t| t.agent_id == agent.id).count();
            let notes = memory_files
                .get("memory/notes.md")
                .map(Vec::as_slice)
                .unwrap_or(&[]);

            agents_json.push(json!({
                "agent_id": agent.id,
                "actions": action_count,
                "memory": {
                    "files": snapshot_json_files(&memory_files),
                    "entries": count_memory_entries(notes),
                    "last_note": last_round_note(notes),
                }
            }));
        }

        let trace: Vec<Value> = self.trace.iter().map(action_trace_json).collect();
        let history: Vec<Value> = self
            .history
            .iter()
            .map(|frame| {
                let frame_cas = cas_metrics_from_actions(&frame.round_actions);
                json!({
                    "frame": frame.frame,
                    "round": frame.round,
                    "label": frame.label,
                    "cas": {
                        "conflicts": frame_cas.conflicts,
                        "retried_writes": frame_cas.retried_writes,
                        "max_retries": frame_cas.max_retries,
                    },
                    "repo": {
                        "total_bytes": frame.repo_total_bytes,
                        "files": frame_file_json(&frame.repo_files),
                    },
                    "skills": {
                        "total_bytes": frame.skill_total_bytes,
                        "files": frame_file_json(&frame.skill_files),
                    },
                    "agents": frame_agent_json(&frame.agents),
                    "round_actions": frame.round_actions.iter().map(action_trace_json).collect::<Vec<_>>(),
                })
            })
            .collect();

        Ok(json!({
            "summary": {
                "seed": summary.seed,
                "profile": summary.profile.as_str(),
                "rounds": summary.rounds,
                "agent_count": summary.agent_count,
                "trace_entries": summary.trace_entries,
                "errors": summary.error_count,
                "cas_conflicts": summary.cas_conflicts,
                "cas_retried_writes": summary.cas_retried_writes,
                "cas_max_retries": summary.cas_max_retries,
                "code_file_count": summary.code_file_count,
                "shared_skill_file_count": summary.shared_skill_file_count,
                "agents_with_memory": summary.agents_with_memory,
                "total_memory_entries": summary.total_memory_entries,
                "code_contributors": summary.code_contributors,
                "skill_contributors": summary.skill_contributors,
            },
            "repo": {
                "files": snapshot_json_files(&repo_files),
            },
            "skills": {
                "files": snapshot_json_files(&skill_files),
            },
            "agents": agents_json,
            "errors": self.errors,
            "trace": trace,
            "history": history,
        }))
    }

    fn make_plan(&mut self, agent_id: usize, round: usize) -> AgentRoundPlan {
        let (code_path, topic) = match self.profile {
            CodingSimProfile::Balanced => {
                let own_code_path = format!("src/agents/agent_{}.rs", agent_id);
                let code_path = if self.rng.gen_bool(0.72) {
                    own_code_path
                } else {
                    self.code_targets[self.rng.gen_range(0..self.code_targets.len())].clone()
                };
                let topic =
                    self.skill_topics[self.rng.gen_range(0..self.skill_topics.len())].clone();
                (code_path, topic)
            }
            CodingSimProfile::CasDemo => {
                let primary_code = [
                    "src/lib.rs",
                    "src/router.rs",
                    "tests/integration_smoke.rs",
                    "src/skills.rs",
                ];
                let primary = primary_code[round % primary_code.len()];
                let secondary = primary_code[(round + 1) % primary_code.len()];
                let code_path = if agent_id % 3 == 0 {
                    primary.to_string()
                } else {
                    secondary.to_string()
                };

                let topic_index = round % self.skill_topics.len();
                let topic = if agent_id % 2 == 0 {
                    self.skill_topics[topic_index].clone()
                } else {
                    self.skill_topics[(topic_index + 1) % self.skill_topics.len()].clone()
                };
                (code_path, topic)
            }
        };

        let skill_path = format!("skills/{}.md", topic);
        let ticket: u32 = self.rng.gen();

        let code_delta = format!(
            "\n// round {} agent {} ticket {}\npub fn agent_{}_round_{}_ticket_{}() -> &'static str {{ \"ticket_{}\" }}\n",
            round,
            agent_id,
            ticket,
            agent_id,
            round,
            ticket % 100_000,
            ticket
        );
        let skill_entry = format!(
            "- round {}: agent {} added guidance for `{}` while editing `{}`\n",
            round, agent_id, topic, code_path
        );
        let memory_entry = format!(
            "- round {}: agent {} edited `{}` and published `{}` (ticket {})\n",
            round, agent_id, code_path, skill_path, ticket
        );

        AgentRoundPlan {
            round,
            agent_id,
            code_path,
            code_expected_token: None,
            code_delta,
            skill_path,
            skill_expected_token: None,
            skill_entry,
            memory_path: "memory/notes.md".to_string(),
            memory_entry,
        }
    }
}

/// Write `coding-agents-report.json` into `output_dir`.
pub async fn write_report(
    sim: &CodingAgentSim,
    output_dir: impl AsRef<Path>,
) -> io::Result<PathBuf> {
    let report = sim
        .report_json()
        .await
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
    let output_dir = output_dir.as_ref();
    std::fs::create_dir_all(output_dir)?;

    let report_path = output_dir.join("coding-agents-report.json");
    std::fs::write(
        &report_path,
        pretty_json_bytes(&report, "coding sim report")?,
    )?;
    Ok(report_path)
}

/// Render an interactive HTML dashboard from a report payload.
pub fn render_html_from_report(report: &Value) -> String {
    let payload = serde_json::to_string(report)
        .unwrap_or_else(|_| "{}".to_string())
        .replace("</script>", "<\\/script>");

    let mut html = String::new();
    html.push_str(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>OpenFS Coding Agent Sim UI</title>
  <style>
    :root {
      --bg: #eef3e9;
      --panel: #fbfff7;
      --ink: #16201a;
      --muted: #52635a;
      --line: #c5d4c8;
      --accent: #1f7a64;
      --accent2: #c9622a;
      --tone0: #2f6ab8;
      --tone1: #1f8f6f;
      --tone2: #9c5bbf;
      --tone3: #c8672f;
      --tone4: #8a4d44;
      --tone5: #43717d;
      --shadow: 0 12px 24px rgba(40, 58, 49, 0.09);
    }

    * { box-sizing: border-box; }

    body {
      margin: 0;
      font-family: "Space Grotesk", "Avenir Next", "Segoe UI", sans-serif;
      color: var(--ink);
      background:
        radial-gradient(1000px 450px at -5% -20%, rgba(31, 122, 100, 0.14), transparent),
        radial-gradient(800px 400px at 110% -10%, rgba(201, 98, 42, 0.13), transparent),
        var(--bg);
    }

    main {
      width: min(1380px, calc(100% - 1.6rem));
      margin: 1rem auto 2.2rem;
      display: grid;
      gap: 0.9rem;
    }

    section {
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 14px;
      padding: 0.9rem;
      box-shadow: var(--shadow);
    }

    .hero {
      background: linear-gradient(130deg, rgba(31, 122, 100, 0.14), rgba(201, 98, 42, 0.14));
    }

    h1 {
      margin: 0;
      font-size: clamp(1.15rem, 2.4vw, 1.75rem);
      letter-spacing: 0.01em;
    }

    h2 {
      margin: 0 0 0.55rem;
      text-transform: uppercase;
      letter-spacing: 0.06em;
      font-size: 0.86rem;
      color: var(--muted);
    }

    h3 {
      margin: 0 0 0.5rem;
      font-size: 0.95rem;
    }

    .sub {
      margin-top: 0.35rem;
      color: var(--muted);
      font-size: 0.93rem;
    }

    .cards {
      margin-top: 0.8rem;
      display: grid;
      gap: 0.55rem;
      grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    }

    .card {
      border: 1px solid var(--line);
      border-radius: 11px;
      background: #fff;
      padding: 0.52rem 0.62rem;
    }

    .k {
      color: var(--muted);
      font-size: 0.72rem;
      text-transform: uppercase;
      letter-spacing: 0.06em;
    }

    .v {
      margin-top: 0.15rem;
      font-weight: 700;
      font-size: 1.1rem;
    }

    .replay-controls {
      display: grid;
      grid-template-columns: auto auto auto minmax(200px, 1fr) auto auto;
      gap: 0.5rem;
      align-items: center;
    }

    .replay-controls button,
    .replay-controls select {
      border: 1px solid var(--line);
      border-radius: 9px;
      background: #fff;
      color: var(--ink);
      padding: 0.36rem 0.55rem;
      font: inherit;
      cursor: pointer;
    }

    .replay-controls input[type="range"] {
      width: 100%;
      accent-color: var(--accent);
    }

    .frame-label {
      justify-self: end;
      font-size: 0.82rem;
      color: var(--muted);
      white-space: nowrap;
    }

    .panes {
      display: grid;
      gap: 0.7rem;
      grid-template-columns: 2fr 2fr 1.5fr;
    }

    .pane {
      border: 1px solid var(--line);
      border-radius: 11px;
      background: #fff;
      padding: 0.6rem;
      min-height: 280px;
      display: flex;
      flex-direction: column;
    }

    .files {
      border: 1px solid #e6efe5;
      border-radius: 9px;
      overflow: auto;
      max-height: 470px;
      background: #fcfffb;
    }

    .file-row {
      display: grid;
      grid-template-columns: minmax(150px, 1fr) minmax(220px, 1.2fr);
      gap: 0.4rem;
      align-items: center;
      border-bottom: 1px solid #e6efe5;
      padding: 0.34rem 0.46rem;
    }

    .file-row:last-child {
      border-bottom: 0;
    }

    .path {
      font-family: "IBM Plex Mono", "Menlo", monospace;
      font-size: 0.77rem;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .meta {
      display: flex;
      gap: 0.28rem;
      flex-wrap: wrap;
      justify-content: flex-end;
    }

    .tag {
      border: 1px solid var(--line);
      border-radius: 999px;
      padding: 0.07rem 0.36rem;
      font-size: 0.67rem;
      white-space: nowrap;
      background: #f4faf5;
    }

    .tag.scope {
      background: rgba(31, 122, 100, 0.15);
      border-color: rgba(31, 122, 100, 0.35);
      color: #135945;
    }

    .agent-chips {
      display: flex;
      gap: 0.22rem;
      flex-wrap: wrap;
      justify-content: flex-end;
      margin-top: 0.18rem;
    }

    .chip {
      border-radius: 999px;
      font-size: 0.62rem;
      padding: 0.07rem 0.34rem;
      border: 1px solid transparent;
      color: #163243;
      background: #e6edf1;
    }

    .chip.mute {
      background: #f3f6f2;
      color: #708078;
      border-color: #d8e0d9;
    }

    .tone-0 { background: color-mix(in srgb, var(--tone0) 16%, white); border-color: color-mix(in srgb, var(--tone0) 36%, white); color: color-mix(in srgb, var(--tone0) 70%, black); }
    .tone-1 { background: color-mix(in srgb, var(--tone1) 16%, white); border-color: color-mix(in srgb, var(--tone1) 36%, white); color: color-mix(in srgb, var(--tone1) 70%, black); }
    .tone-2 { background: color-mix(in srgb, var(--tone2) 16%, white); border-color: color-mix(in srgb, var(--tone2) 36%, white); color: color-mix(in srgb, var(--tone2) 70%, black); }
    .tone-3 { background: color-mix(in srgb, var(--tone3) 16%, white); border-color: color-mix(in srgb, var(--tone3) 36%, white); color: color-mix(in srgb, var(--tone3) 70%, black); }
    .tone-4 { background: color-mix(in srgb, var(--tone4) 16%, white); border-color: color-mix(in srgb, var(--tone4) 36%, white); color: color-mix(in srgb, var(--tone4) 70%, black); }
    .tone-5 { background: color-mix(in srgb, var(--tone5) 16%, white); border-color: color-mix(in srgb, var(--tone5) 36%, white); color: color-mix(in srgb, var(--tone5) 70%, black); }

    .edit-bar {
      margin-top: 0.25rem;
      width: 100%;
      height: 4px;
      border-radius: 999px;
      background: #ecf2ec;
      overflow: hidden;
    }

    .edit-bar > span {
      display: block;
      height: 100%;
      background: linear-gradient(90deg, var(--accent), #46a68f);
    }

    .agent-grid {
      display: grid;
      gap: 0.45rem;
      grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    }

    .agent-card {
      border: 1px solid var(--line);
      border-radius: 10px;
      background: #fff;
      padding: 0.45rem 0.5rem;
    }

    .agent-note {
      margin-top: 0.32rem;
      font-family: "IBM Plex Mono", "Menlo", monospace;
      font-size: 0.69rem;
      color: var(--muted);
      line-height: 1.35;
      max-height: 3.2em;
      overflow: hidden;
    }

    table {
      width: 100%;
      border-collapse: collapse;
      font-family: "IBM Plex Mono", "Menlo", monospace;
      font-size: 0.76rem;
      margin-top: 0.35rem;
    }

    th, td {
      border-bottom: 1px solid #e6efe5;
      padding: 0.27rem 0.32rem;
      text-align: left;
      vertical-align: top;
    }

    th {
      color: var(--muted);
      font-family: "Space Grotesk", "Avenir Next", sans-serif;
      font-size: 0.7rem;
      text-transform: uppercase;
      letter-spacing: 0.06em;
    }

    tr.active-round {
      background: rgba(31, 122, 100, 0.09);
    }

    tr.retry-row {
      background: rgba(201, 98, 42, 0.11);
    }

    .pill {
      display: inline-block;
      margin-left: 0.3rem;
      border: 1px solid transparent;
      border-radius: 999px;
      padding: 0.04rem 0.34rem;
      font-size: 0.6rem;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      vertical-align: middle;
      white-space: nowrap;
    }

    .pill.cas {
      color: #7a3d14;
      background: rgba(201, 98, 42, 0.18);
      border-color: rgba(201, 98, 42, 0.33);
    }

    .empty {
      color: var(--muted);
      font-size: 0.82rem;
      padding: 0.45rem 0.2rem;
    }

    @media (max-width: 1060px) {
      .panes {
        grid-template-columns: 1fr;
      }
    }

    @media (max-width: 680px) {
      .replay-controls {
        grid-template-columns: repeat(3, auto);
      }
      .replay-controls input[type="range"] {
        grid-column: 1 / -1;
      }
      .frame-label {
        justify-self: start;
      }
    }
  </style>
</head>
<body>
  <main>
    <section class="hero">
      <h1>OpenFS Coding Agent Sim UI</h1>
      <div class="sub">Replay concurrent coding rounds with shared code, shared skills, and private agent memory.</div>
      <div class="cards" id="summary-cards"></div>
    </section>

    <section>
      <h2>Replay</h2>
      <div class="replay-controls">
        <button id="step-back" type="button">Back</button>
        <button id="play-toggle" type="button">Play</button>
        <button id="step-forward" type="button">Forward</button>
        <input id="replay-slider" type="range" min="0" max="0" value="0" />
        <select id="play-speed">
          <option value="850">0.5x</option>
          <option value="420" selected>1x</option>
          <option value="170">2.5x</option>
          <option value="85">5x</option>
        </select>
        <div class="frame-label" id="frame-label"></div>
      </div>
      <div class="cards" id="frame-cards"></div>
    </section>

    <section class="panes">
      <div class="pane">
        <h3>Shared Repo Files</h3>
        <div id="repo-files" class="files"></div>
      </div>
      <div class="pane">
        <h3>Shared Skill Files</h3>
        <div id="skill-files" class="files"></div>
      </div>
      <div class="pane">
        <h3>Agent Memory</h3>
        <div id="agent-memory" class="agent-grid"></div>
      </div>
    </section>

    <section>
      <h2>Round Actions</h2>
      <table>
        <thead>
          <tr>
            <th>Round</th>
            <th>Agent</th>
            <th>Action</th>
            <th>Path</th>
            <th>Bytes</th>
          </tr>
        </thead>
        <tbody id="round-actions"></tbody>
      </table>
    </section>

    <section>
      <h2>Full Timeline</h2>
      <table>
        <thead>
          <tr>
            <th>Round</th>
            <th>Agent</th>
            <th>Action</th>
            <th>Path</th>
            <th>Bytes</th>
          </tr>
        </thead>
        <tbody id="trace"></tbody>
      </table>
    </section>
  </main>

  <script id="coding-sim-data" type="application/json">"#,
    );
    html.push_str(&payload);
    html.push_str(
        r#"</script>
  <script>
    const report = JSON.parse(document.getElementById("coding-sim-data").textContent || "{}");
    const summary = report.summary || {};
    const history = Array.isArray(report.history) ? report.history : [];
    const trace = Array.isArray(report.trace) ? report.trace : [];

    let frameIndex = history.length > 0 ? history.length - 1 : 0;
    let playTimer = null;

    const esc = (value) => {
      if (value === null || value === undefined) return "";
      return String(value)
        .replaceAll("&", "&amp;")
        .replaceAll("<", "&lt;")
        .replaceAll(">", "&gt;")
        .replaceAll('"', "&quot;");
    };

    const intValue = (value) => Number.isFinite(Number(value)) ? Number(value) : 0;
    const byId = (id) => document.getElementById(id);
    const isRetryAction = (action) => String(action || "").endsWith("_retry");

    function renderCards(targetId, items) {
      byId(targetId).innerHTML = items
        .map((item) => `<div class="card"><div class="k">${esc(item.k)}</div><div class="v">${esc(item.v)}</div></div>`)
        .join("");
    }

    function contributorChips(contributors) {
      if (!Array.isArray(contributors) || contributors.length === 0) {
        return '<span class="chip mute">none</span>';
      }
      return contributors
        .map((id) => `<span class="chip tone-${intValue(id) % 6}">A${esc(id)}</span>`)
        .join("");
    }

    function renderFileList(targetId, files, scopeLabel) {
      if (!Array.isArray(files) || files.length === 0) {
        byId(targetId).innerHTML = '<div class="empty">No files in this frame.</div>';
        return;
      }

      const rows = files
        .map((file) => {
          const edits = intValue(file.edits);
          const width = Math.min(100, edits * 9 + 2);
          const lastAgent = file.last_agent === null || file.last_agent === undefined
            ? "none"
            : `A${file.last_agent}`;
          return `
            <div class="file-row">
              <div>
                <div class="path">${esc(file.path)}</div>
                <div class="edit-bar"><span style="width:${width}%"></span></div>
              </div>
              <div>
                <div class="meta">
                  <span class="tag scope">${esc(scopeLabel)}</span>
                  <span class="tag">${intValue(file.bytes)} bytes</span>
                  <span class="tag">${edits} edits</span>
                  <span class="tag">last ${esc(lastAgent)}</span>
                </div>
                <div class="agent-chips">${contributorChips(file.contributors)}</div>
              </div>
            </div>
          `;
        })
        .join("");

      byId(targetId).innerHTML = rows;
    }

    function renderAgentMemory(agents) {
      if (!Array.isArray(agents) || agents.length === 0) {
        byId("agent-memory").innerHTML = '<div class="empty">No agent memory in this frame.</div>';
        return;
      }

      byId("agent-memory").innerHTML = agents
        .map((agent) => `
          <div class="agent-card">
            <div class="k">Agent ${esc(agent.agent_id)}</div>
            <div class="v">${intValue(agent.memory_entries)} memory entries</div>
            <div class="meta">
              <span class="tag">${intValue(agent.memory_bytes)} bytes</span>
              <span class="tag">${intValue(agent.cumulative_actions)} actions</span>
            </div>
            <div class="agent-note">${esc(agent.last_note || "no round note yet")}</div>
          </div>
        `)
        .join("");
    }

    function renderActions(targetId, actions) {
      if (!Array.isArray(actions) || actions.length === 0) {
        byId(targetId).innerHTML = '<tr><td colspan="5" class="empty">No actions for this frame.</td></tr>';
        return;
      }

      byId(targetId).innerHTML = actions
        .map((row) => {
          const retry = isRetryAction(row.action);
          return `
          <tr class="${retry ? "retry-row" : ""}">
            <td>${esc(row.round)}</td>
            <td>A${esc(row.agent_id)}</td>
            <td>${esc(row.action)}${retry ? '<span class="pill cas">CAS retry</span>' : ""}</td>
            <td>${esc(row.path)}</td>
            <td>${esc(row.bytes)}</td>
          </tr>
        `;
        })
        .join("");
    }

    function renderTimeline(currentRound) {
      if (!Array.isArray(trace) || trace.length === 0) {
        byId("trace").innerHTML = '<tr><td colspan="5" class="empty">No trace entries.</td></tr>';
        return;
      }

      byId("trace").innerHTML = trace
        .map((row) => {
          const retry = isRetryAction(row.action);
          const activeClass = intValue(row.round) === intValue(currentRound) ? "active-round" : "";
          const rowClass = `${activeClass} ${retry ? "retry-row" : ""}`.trim();
          return `
            <tr class="${rowClass}">
              <td>${esc(row.round)}</td>
              <td>A${esc(row.agent_id)}</td>
              <td>${esc(row.action)}${retry ? '<span class="pill cas">CAS retry</span>' : ""}</td>
              <td>${esc(row.path)}</td>
              <td>${esc(row.bytes)}</td>
            </tr>
          `;
        })
        .join("");
    }

    function updateFrame() {
      if (history.length === 0) {
        byId("frame-label").textContent = "no frames";
        renderCards("frame-cards", []);
        renderFileList("repo-files", [], "REPO");
        renderFileList("skill-files", [], "SKILL");
        renderAgentMemory([]);
        renderActions("round-actions", []);
        renderTimeline(-1);
        return;
      }

      const frame = history[frameIndex] || history[history.length - 1];
      const repoFiles = Array.isArray(frame.repo && frame.repo.files) ? frame.repo.files : [];
      const skillFiles = Array.isArray(frame.skills && frame.skills.files) ? frame.skills.files : [];
      const agents = Array.isArray(frame.agents) ? frame.agents : [];
      const actions = Array.isArray(frame.round_actions) ? frame.round_actions : [];
      const frameCas = frame.cas || {};

      byId("replay-slider").max = String(Math.max(history.length - 1, 0));
      byId("replay-slider").value = String(frameIndex);
      byId("frame-label").textContent = `${frame.label || `round ${frame.round}`} (${frameIndex + 1}/${history.length})`;

      renderCards("frame-cards", [
        { k: "Frame", v: frameIndex + 1 },
        { k: "Round", v: frame.round },
        { k: "Repo Files", v: repoFiles.length },
        { k: "Skill Files", v: skillFiles.length },
        { k: "Round Actions", v: actions.length },
        { k: "CAS Conflicts", v: intValue(frameCas.conflicts) },
        { k: "CAS Retries", v: intValue(frameCas.retried_writes) },
        { k: "Max CAS Retry", v: intValue(frameCas.max_retries) },
        { k: "Memory Entries", v: agents.reduce((sum, a) => sum + intValue(a.memory_entries), 0) },
      ]);

      renderFileList("repo-files", repoFiles, "REPO");
      renderFileList("skill-files", skillFiles, "SKILL");
      renderAgentMemory(agents);
      renderActions("round-actions", actions);
      renderTimeline(frame.round);
    }

    function stopPlay() {
      if (playTimer !== null) {
        clearInterval(playTimer);
        playTimer = null;
      }
      byId("play-toggle").textContent = "Play";
    }

    function startPlay() {
      if (history.length === 0) return;
      if (frameIndex >= history.length - 1) frameIndex = 0;

      const delay = Math.max(40, intValue(byId("play-speed").value) || 420);
      playTimer = setInterval(() => {
        if (frameIndex >= history.length - 1) {
          stopPlay();
          return;
        }
        frameIndex += 1;
        updateFrame();
      }, delay);
      byId("play-toggle").textContent = "Pause";
      updateFrame();
    }

    byId("step-back").addEventListener("click", () => {
      stopPlay();
      frameIndex = Math.max(0, frameIndex - 1);
      updateFrame();
    });

    byId("step-forward").addEventListener("click", () => {
      stopPlay();
      frameIndex = Math.min(Math.max(history.length - 1, 0), frameIndex + 1);
      updateFrame();
    });

    byId("replay-slider").addEventListener("input", (event) => {
      stopPlay();
      frameIndex = intValue(event.target.value);
      updateFrame();
    });

    byId("play-toggle").addEventListener("click", () => {
      if (playTimer === null) startPlay();
      else stopPlay();
    });

    byId("play-speed").addEventListener("change", () => {
      if (playTimer !== null) {
        stopPlay();
        startPlay();
      }
    });

    renderCards("summary-cards", [
      { k: "Seed", v: summary.seed },
      { k: "Profile", v: summary.profile || "balanced" },
      { k: "Rounds", v: summary.rounds },
      { k: "Agents", v: summary.agent_count },
      { k: "Trace Entries", v: summary.trace_entries },
      { k: "CAS Conflicts", v: summary.cas_conflicts },
      { k: "CAS Retries", v: summary.cas_retried_writes },
      { k: "Max CAS Retry", v: summary.cas_max_retries },
      { k: "Code Files", v: summary.code_file_count },
      { k: "Skill Files", v: summary.shared_skill_file_count },
      { k: "Code Contributors", v: summary.code_contributors },
      { k: "Skill Contributors", v: summary.skill_contributors },
      { k: "Memory Entries", v: summary.total_memory_entries },
      { k: "Errors", v: summary.errors },
    ]);

    updateFrame();
  </script>
</body>
</html>
"#,
    );
    html
}

/// Write `coding-agents-ui.html` and `coding-agents-data.json` into `output_dir`.
pub async fn write_bundle(
    sim: &CodingAgentSim,
    output_dir: impl AsRef<Path>,
) -> io::Result<(PathBuf, PathBuf)> {
    let report = sim
        .report_json()
        .await
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;

    let output_dir = output_dir.as_ref();
    std::fs::create_dir_all(output_dir)?;

    let html_path = output_dir.join("coding-agents-ui.html");
    let json_path = output_dir.join("coding-agents-data.json");

    std::fs::write(
        &json_path,
        pretty_json_bytes(&report, "coding sim bundle data")?,
    )?;
    std::fs::write(&html_path, render_html_from_report(&report))?;

    Ok((html_path, json_path))
}

async fn execute_agent_plan(
    agent: CodingAgentVm,
    plan: AgentRoundPlan,
) -> Result<Vec<CodingActionTrace>, String> {
    let mut actions = Vec::with_capacity(6);

    let code_retries = cas_append(
        agent.repo_mount.as_ref(),
        &plan.code_path,
        plan.code_delta.as_bytes(),
        plan.code_expected_token.as_deref(),
    )
    .await
    .map_err(|err| {
        format!(
            "{} ({})",
            format_backend_error(plan.round, plan.agent_id, "edit_code", &plan.code_path, err),
            "compare_and_swap"
        )
    })?;
    actions.push(CodingActionTrace {
        round: plan.round,
        agent_id: plan.agent_id,
        action: "edit_code".to_string(),
        path: plan.code_path.clone(),
        bytes: plan.code_delta.len(),
    });
    if code_retries > 0 {
        actions.push(CodingActionTrace {
            round: plan.round,
            agent_id: plan.agent_id,
            action: "edit_code_retry".to_string(),
            path: plan.code_path.clone(),
            bytes: code_retries,
        });
    }

    let skill_retries = cas_append(
        agent.skills_mount.as_ref(),
        &plan.skill_path,
        plan.skill_entry.as_bytes(),
        plan.skill_expected_token.as_deref(),
    )
    .await
    .map_err(|err| {
        format!(
            "{} ({})",
            format_backend_error(
                plan.round,
                plan.agent_id,
                "publish_skill",
                &plan.skill_path,
                err,
            ),
            "compare_and_swap"
        )
    })?;
    actions.push(CodingActionTrace {
        round: plan.round,
        agent_id: plan.agent_id,
        action: "publish_skill".to_string(),
        path: plan.skill_path.clone(),
        bytes: plan.skill_entry.len(),
    });
    if skill_retries > 0 {
        actions.push(CodingActionTrace {
            round: plan.round,
            agent_id: plan.agent_id,
            action: "publish_skill_retry".to_string(),
            path: plan.skill_path.clone(),
            bytes: skill_retries,
        });
    }

    let memory_retries = cas_append(
        agent.memory_mount.as_ref(),
        &plan.memory_path,
        plan.memory_entry.as_bytes(),
        None,
    )
    .await
    .map_err(|err| {
        format!(
            "{} ({})",
            format_backend_error(
                plan.round,
                plan.agent_id,
                "save_memory",
                &plan.memory_path,
                err,
            ),
            "compare_and_swap"
        )
    })?;
    actions.push(CodingActionTrace {
        round: plan.round,
        agent_id: plan.agent_id,
        action: "save_memory".to_string(),
        path: plan.memory_path.clone(),
        bytes: plan.memory_entry.len(),
    });
    if memory_retries > 0 {
        actions.push(CodingActionTrace {
            round: plan.round,
            agent_id: plan.agent_id,
            action: "save_memory_retry".to_string(),
            path: plan.memory_path.clone(),
            bytes: memory_retries,
        });
    }

    Ok(actions)
}

async fn cas_append(
    backend: &CachedBackend<DynBackend>,
    path: &str,
    delta: &[u8],
    seeded_expected_token: Option<&str>,
) -> Result<usize, BackendError> {
    const MAX_RETRIES: usize = 8;
    let mut first_expected = seeded_expected_token.map(ToString::to_string);

    for attempt in 0..=MAX_RETRIES {
        let (current, token) = match backend.read_with_cas_token(path).await {
            Ok(value) => value,
            Err(BackendError::NotFound(_)) => (Vec::new(), None),
            Err(err) => return Err(err),
        };

        let expected = if attempt == 0 {
            first_expected.take().or(token.clone())
        } else {
            token.clone()
        };

        let mut next = current;
        next.extend_from_slice(delta);

        match backend
            .compare_and_swap(path, expected.as_deref(), &next)
            .await
        {
            Ok(_) => return Ok(attempt),
            Err(BackendError::PreconditionFailed { .. }) if attempt < MAX_RETRIES => continue,
            Err(err) => return Err(err),
        }
    }

    Err(BackendError::Other(format!(
        "CAS retries exhausted for path {}",
        path
    )))
}

fn sanitize_ident(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn action_rank(action: &str) -> usize {
    match action {
        "edit_code" => 0,
        "edit_code_retry" => 1,
        "publish_skill" => 2,
        "publish_skill_retry" => 3,
        "save_memory" => 4,
        "save_memory_retry" => 5,
        _ => 9,
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct CasActionMetrics {
    conflicts: usize,
    retried_writes: usize,
    max_retries: usize,
}

fn is_retry_action(action: &str) -> bool {
    matches!(
        action,
        "edit_code_retry" | "publish_skill_retry" | "save_memory_retry"
    )
}

fn cas_metrics_from_actions(actions: &[CodingActionTrace]) -> CasActionMetrics {
    let mut metrics = CasActionMetrics::default();

    for action in actions {
        if !is_retry_action(&action.action) {
            continue;
        }
        let retries = action.bytes;
        if retries == 0 {
            continue;
        }
        metrics.retried_writes += 1;
        metrics.conflicts += retries;
        metrics.max_retries = metrics.max_retries.max(retries);
    }

    metrics
}

fn action_trace_json(t: &CodingActionTrace) -> Value {
    json!({
        "round": t.round,
        "agent_id": t.agent_id,
        "action": t.action,
        "path": t.path,
        "bytes": t.bytes,
    })
}

fn frame_file_json(files: &[CodingFrameFile]) -> Vec<Value> {
    files
        .iter()
        .map(|file| {
            json!({
                "path": file.path,
                "bytes": file.bytes,
                "contributors": file.contributors,
                "last_agent": file.last_agent,
                "edits": file.edits,
            })
        })
        .collect()
}

fn frame_agent_json(agents: &[CodingFrameAgent]) -> Vec<Value> {
    agents
        .iter()
        .map(|agent| {
            json!({
                "agent_id": agent.agent_id,
                "memory_entries": agent.memory_entries,
                "memory_bytes": agent.memory_bytes,
                "last_note": agent.last_note,
                "cumulative_actions": agent.cumulative_actions,
            })
        })
        .collect()
}

fn snapshot_frame_files(
    files: &BTreeMap<String, Vec<u8>>,
    counter_kind: EditCounterKind,
) -> (Vec<CodingFrameFile>, usize) {
    let mut total_bytes = 0usize;
    let rows = files
        .iter()
        .map(|(path, content)| {
            total_bytes += content.len();
            let contributors = contributor_ids_from_bytes(content)
                .into_iter()
                .collect::<Vec<_>>();
            let last_agent = last_agent_id_from_bytes(content);
            let edits = match counter_kind {
                EditCounterKind::Code => count_code_edits(content),
                EditCounterKind::Bullets => count_round_entries(content),
            };

            CodingFrameFile {
                path: path.clone(),
                bytes: content.len(),
                contributors,
                last_agent,
                edits,
            }
        })
        .collect();
    (rows, total_bytes)
}

fn format_backend_error(
    round: usize,
    agent_id: usize,
    action: &str,
    path: &str,
    err: BackendError,
) -> String {
    format!(
        "round {} agent {} action={} path={} error={}",
        round, agent_id, action, path, err
    )
}

async fn read_backend_cas_token(
    backend: &dyn Backend,
    path: &str,
) -> Result<Option<String>, String> {
    match backend.read_with_cas_token(path).await {
        Ok((_content, token)) => Ok(token),
        Err(BackendError::NotFound(_)) => Ok(None),
        Err(err) => Err(format!(
            "failed to read CAS token for path {}: {}",
            path, err
        )),
    }
}

async fn collect_backend_files(backend: &dyn Backend) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut files = BTreeMap::new();
    let mut stack = vec![String::new()];
    let mut visited = BTreeSet::new();

    while let Some(dir) = stack.pop() {
        if !visited.insert(dir.clone()) {
            continue;
        }

        let mut entries = backend.list(&dir).await.map_err(|err| err.to_string())?;
        entries.sort_by(|a, b| a.path.cmp(&b.path));

        for entry in entries.into_iter().rev() {
            let path = entry.path.trim_matches('/').to_string();
            if path.is_empty() {
                continue;
            }
            if entry.is_dir {
                stack.push(path);
            } else {
                let data = backend.read(&path).await.map_err(|err| err.to_string())?;
                files.insert(path, data);
            }
        }
    }

    Ok(files)
}

fn snapshot_json_files(files: &BTreeMap<String, Vec<u8>>) -> Vec<Value> {
    files
        .iter()
        .map(|(path, content)| {
            json!({
                "path": path,
                "bytes": content.len(),
                "preview": preview(content),
            })
        })
        .collect()
}

fn pretty_json_bytes(payload: &Value, label: &str) -> io::Result<Vec<u8>> {
    serde_json::to_vec_pretty(payload).map_err(|err| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("failed to serialize {}: {}", label, err),
        )
    })
}

fn preview(content: &[u8]) -> String {
    let text = String::from_utf8_lossy(content);
    let mut line = text.lines().next().unwrap_or_default().to_string();
    if line.len() > 120 {
        line.truncate(120);
    }
    line
}

fn count_code_edits(content: &[u8]) -> usize {
    String::from_utf8_lossy(content)
        .lines()
        .filter(|line| line.starts_with("// round "))
        .count()
}

fn count_round_entries(content: &[u8]) -> usize {
    String::from_utf8_lossy(content)
        .lines()
        .filter(|line| line.starts_with("- round "))
        .count()
}

fn count_memory_entries(content: &[u8]) -> usize {
    count_round_entries(content)
}

fn last_round_note(content: &[u8]) -> String {
    let mut note = String::new();
    for line in String::from_utf8_lossy(content).lines() {
        if line.starts_with("- round ") {
            note = line.to_string();
        }
    }
    note
}

fn contributor_ids_in_files(files: &BTreeMap<String, Vec<u8>>) -> BTreeSet<usize> {
    let mut ids = BTreeSet::new();
    for content in files.values() {
        for id in contributor_ids_from_bytes(content) {
            ids.insert(id);
        }
    }
    ids
}

fn contributor_ids_from_bytes(content: &[u8]) -> BTreeSet<usize> {
    let text = String::from_utf8_lossy(content);
    let mut ids = BTreeSet::new();
    let mut expect_id = false;

    for token in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        if token.is_empty() {
            continue;
        }

        if expect_id {
            if let Ok(id) = token.parse::<usize>() {
                ids.insert(id);
            }
            expect_id = false;
            continue;
        }

        if token == "agent" {
            expect_id = true;
        }
    }

    ids
}

fn last_agent_id_from_bytes(content: &[u8]) -> Option<usize> {
    let text = String::from_utf8_lossy(content);
    let mut last_id = None;
    let mut expect_id = false;

    for token in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        if token.is_empty() {
            continue;
        }

        if expect_id {
            if let Ok(id) = token.parse::<usize>() {
                last_id = Some(id);
            }
            expect_id = false;
            continue;
        }

        if token == "agent" {
            expect_id = true;
        }
    }

    last_id
}
