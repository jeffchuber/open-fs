use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use openfs_core::{Backend, BackendError, ChromaStore, VfsError};
use openfs_local::{IndexingPipeline, PipelineConfig};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use crate::agent::{build_agents, AgentVm};
use crate::fault::{is_injected_fault, FaultConfig};
use crate::invariants::{check_final_consistency, check_step_invariants, Violation};
use crate::mock_chroma::MockChromaStore;
use crate::ops::{generate, AgentOpState, EntrySummary, MountId, Op};
use crate::oracle::{Expected, Oracle};
use openfs_remote::MemoryBackend;

/// A single operation/result entry captured during simulation.
#[derive(Debug, Clone)]
pub struct SimTraceEntry {
    pub step: usize,
    pub agent_id: usize,
    pub op: String,
    pub expected: String,
    pub actual: String,
    pub note: Option<String>,
}

/// Sync counters captured for a single agent at a point in time.
#[derive(Debug, Clone)]
pub struct SimSyncFrame {
    pub mode: String,
    pub pending: usize,
    pub synced: u64,
    pub failed: u64,
    pub retries: u64,
    pub cache_entries: usize,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub wal_present: bool,
    pub wal_pending: usize,
    pub wal_processing: usize,
    pub wal_failed: usize,
    pub wal_unapplied: usize,
    pub wal_paths_pending: Vec<String>,
    pub wal_paths_processing: Vec<String>,
    pub wal_paths_failed: Vec<String>,
}

/// Per-agent state captured for timeline replay.
#[derive(Debug, Clone)]
pub struct SimAgentFrame {
    pub agent_id: usize,
    pub work_paths: Vec<String>,
    pub indexed_paths: Vec<String>,
    pub shared_write_paths: Vec<String>,
    pub remote_indexed_paths: Vec<String>,
    pub remote_indexed_error: Option<String>,
    pub sync: Option<SimSyncFrame>,
}

/// A replayable point-in-time snapshot of simulation state.
#[derive(Debug, Clone)]
pub struct SimStateFrame {
    pub frame: usize,
    pub step: usize,
    pub label: Option<String>,
    pub violations: usize,
    pub pending_write_back_paths: Vec<String>,
    pub remote0_paths: Vec<String>,
    pub remote0_error: Option<String>,
    pub agents: Vec<SimAgentFrame>,
}

/// The main simulation harness.
pub struct Sim {
    pub agents: Vec<AgentVm>,
    pub oracle: Oracle,
    pub rng: ChaCha8Rng,
    pub step: usize,
    pub violations: Vec<Violation>,
    pub agent_states: Vec<AgentOpState>,
    pub pipeline: IndexingPipeline,
    /// Whether fault injection is active.
    pub has_faults: bool,
    /// Whether agents 1/2 share one indexed backing store (`remote0`).
    pub indexed_shared_12: bool,
    /// Paths written via write-back (agent 1's indexed mount) but not yet flushed.
    pub pending_write_back_paths: HashSet<String>,
    /// Step-by-step operation outcomes for debug UIs and tooling.
    pub trace: Vec<SimTraceEntry>,
    /// Replayable snapshots captured over time.
    pub history: Vec<SimStateFrame>,
}

impl Sim {
    /// Create a new deterministic simulation with the given seed.
    pub async fn new(seed: u64) -> Self {
        Self::new_with_config(seed, None, false).await
    }

    /// Create a new deterministic simulation with optional fault injection.
    pub async fn new_with_faults(seed: u64, fault_config: Option<FaultConfig>) -> Self {
        Self::new_with_config(seed, fault_config, false).await
    }

    /// Create a simulation that includes a third remote-writer client (agent 2).
    pub async fn new_with_remote_client(
        seed: u64,
        fault_config: Option<FaultConfig>,
        enable_write_back: bool,
    ) -> Self {
        Self::new_with_topology(seed, fault_config, enable_write_back, true).await
    }

    /// Create a new deterministic simulation with full configuration.
    pub async fn new_with_config(
        seed: u64,
        fault_config: Option<FaultConfig>,
        enable_write_back: bool,
    ) -> Self {
        Self::new_with_topology(seed, fault_config, enable_write_back, false).await
    }

    async fn new_with_topology(
        seed: u64,
        fault_config: Option<FaultConfig>,
        enable_write_back: bool,
        include_remote_writer: bool,
    ) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);

        // Create shared backends
        let shared_read = Arc::new(MemoryBackend::new());
        let shared_write = Arc::new(MemoryBackend::new());
        let chroma = Arc::new(MockChromaStore::new("sim_collection"));

        // Seed shared_read with some files
        let mut seed_files = HashMap::new();
        for i in 0..5 {
            let name = format!("seed_{}.txt", i);
            let content = format!("seed_content_{}", i).into_bytes();
            shared_read.write(&name, &content).await.unwrap();
            seed_files.insert(name.clone(), content);
        }

        let mut oracle = Oracle::new_with_indexed_shared_12(include_remote_writer);
        oracle.seed_shared_read(seed_files.clone());

        let has_faults = fault_config.is_some();

        // Build agents
        let agents = build_agents(
            shared_read,
            shared_write,
            chroma.clone(),
            fault_config,
            enable_write_back,
            include_remote_writer,
            &mut rng,
        )
        .await;

        // Build agent op states
        let mut agent_states: Vec<AgentOpState> =
            (0..agents.len()).map(AgentOpState::new).collect();

        // All agents know about seed files in shared/read
        for name in seed_files.keys() {
            for state in &mut agent_states {
                state.add_file(MountId::SharedRead, name.clone());
            }
        }

        // Create indexing pipeline with stub embedder
        let pipeline_config = PipelineConfig {
            embedder_provider: "stub".to_string(),
            chunker_strategy: "fixed".to_string(),
            enable_sparse: true,
            ..Default::default()
        };
        let pipeline = IndexingPipeline::new(pipeline_config)
            .unwrap()
            .with_chroma(chroma as Arc<dyn openfs_core::ChromaStore>);

        let mut sim = Sim {
            agents,
            oracle,
            rng,
            step: 0,
            violations: Vec::new(),
            agent_states,
            pipeline,
            has_faults,
            indexed_shared_12: include_remote_writer,
            pending_write_back_paths: HashSet::new(),
            trace: Vec::new(),
            history: Vec::new(),
        };

        sim.capture_history_snapshot(0, Some("initial")).await;
        sim
    }

    /// Shutdown all agents (flush write-back sync engines).
    pub async fn shutdown(&self) {
        for agent in &self.agents {
            agent.shutdown().await;
        }
        // Give background tasks time to complete
        tokio::time::advance(Duration::from_secs(3)).await;
        tokio::task::yield_now().await;
    }

    /// Run the simulation for the given number of steps.
    pub async fn run(&mut self, steps: usize) -> &[Violation] {
        use rand::Rng;

        for _ in 0..steps {
            // 1. Pick agent
            let agent_id: usize = self.rng.gen_range(0..self.agents.len());

            // 2. Generate operation
            let op = generate(&mut self.rng, &self.agent_states[agent_id], self.step);

            let _ = self.step_with(agent_id, op).await;
        }

        // Shutdown write-back engines before final checks
        self.shutdown().await;
        self.capture_history_snapshot(self.step, Some("post-shutdown"))
            .await;

        let final_violations = check_final_consistency(&self.agents, &self.oracle).await;
        self.violations.extend(final_violations);
        self.capture_history_snapshot(self.step, Some("final"))
            .await;

        &self.violations
    }

    /// Run the simulation mixing sequential and concurrent steps.
    pub async fn run_mixed(&mut self, steps: usize, concurrent_ratio: f64) -> &[Violation] {
        use rand::Rng;

        let ratio = if concurrent_ratio.is_finite() {
            concurrent_ratio.clamp(0.0, 1.0)
        } else {
            0.0
        };

        for _ in 0..steps {
            if self.rng.gen_bool(ratio) {
                let _ = self.step_concurrent().await;
            } else {
                let agent_id: usize = self.rng.gen_range(0..self.agents.len());
                let op = generate(&mut self.rng, &self.agent_states[agent_id], self.step);
                let _ = self.step_with(agent_id, op).await;
            }
        }

        self.shutdown().await;
        self.capture_history_snapshot(self.step, Some("post-shutdown"))
            .await;

        let final_violations = check_final_consistency(&self.agents, &self.oracle).await;
        self.violations.extend(final_violations);
        self.capture_history_snapshot(self.step, Some("final"))
            .await;

        &self.violations
    }

    /// Execute a single operation and return any violations found for that step.
    pub async fn step_with(&mut self, agent_id: usize, op: Op) -> Vec<Violation> {
        assert!(agent_id < self.agents.len(), "invalid agent_id");

        let mut new_violations = Vec::new();

        // Predict expected outcome (read-only)
        let expected = self.oracle.predict(agent_id, &op);

        // Execute against real OpenFS code
        let actual = self.execute(agent_id, &op).await;

        // Check if this was an injected fault (error path)
        let was_fault = matches!(&actual, Outcome::Error(msg) if is_injected_fault(msg));

        if was_fault {
            self.push_trace(
                self.step,
                agent_id,
                &op,
                &expected,
                &actual,
                Some("fault-injected"),
            );
            // Don't commit oracle, don't update agent_state, don't check outcome.
            // Just record the fault and move on.
            self.step += 1;
            self.capture_history_snapshot(self.step, Some("fault-injected"))
                .await;
            return new_violations;
        }

        // Check for write-back mismatches: ops that go to inner backend (stat, list,
        // rename) will fail when data is only in write-back cache.
        let is_write_back_mismatch = agent_id == 1
            && is_write_back_affected_op(&op)
            && write_back_op_touches_pending(&op, &self.pending_write_back_paths)
            && matches!(&actual, Outcome::NotFound | Outcome::Error(_));

        if is_write_back_mismatch {
            self.push_trace(
                self.step,
                agent_id,
                &op,
                &expected,
                &actual,
                Some("write-back-pending-mismatch"),
            );
            // Don't commit oracle or update agent state — the op didn't actually
            // succeed, and committing would cause permanent oracle/backend divergence.
            self.step += 1;
            self.capture_history_snapshot(self.step, Some("write-back-pending-mismatch"))
                .await;
            return new_violations;
        }

        // Commit oracle state change
        self.oracle.commit(agent_id, &op);

        // Compare (skip read outcome checks when faults are enabled, since reads may be corrupted)
        if !(self.has_faults && matches!(op, Op::Read { .. }))
            && !self.relax_shared_indexed_outcome(agent_id, &op)
        {
            if let Some(v) = check_outcome(self.step, agent_id, &op, &expected, &actual) {
                self.violations.push(v.clone());
                new_violations.push(v);
            }
        }

        // Update agent op state based on outcome
        self.update_agent_state(agent_id, &op, &expected);

        // Track write-back pending paths
        self.track_write_back(agent_id, &op, &expected);

        // Strong post-conditions for local consistency (skip if faults active)
        if !self.has_faults {
            if let Some(v) = self
                .verify_post_conditions(self.step, agent_id, &op, &expected)
                .await
            {
                self.violations.push(v.clone());
                new_violations.push(v);
            }
        }

        // Run invariant checks
        let step_violations = check_step_invariants(
            self.step,
            &self.agents,
            &self.oracle,
            &self.pending_write_back_paths,
            self.has_faults,
        )
        .await;
        self.violations.extend(step_violations.clone());
        new_violations.extend(step_violations);

        self.push_trace(self.step, agent_id, &op, &expected, &actual, None);
        self.step += 1;
        self.capture_history_snapshot(self.step, None).await;
        new_violations
    }

    fn push_trace(
        &mut self,
        step: usize,
        agent_id: usize,
        op: &Op,
        expected: &Expected,
        actual: &Outcome,
        note: Option<&str>,
    ) {
        self.trace.push(SimTraceEntry {
            step,
            agent_id,
            op: op_summary(op),
            expected: expected_summary(expected),
            actual: outcome_summary(actual),
            note: note.map(ToString::to_string),
        });
    }

    async fn capture_history_snapshot(&mut self, step: usize, label: Option<&str>) {
        let mut pending_write_back_paths: Vec<String> =
            self.pending_write_back_paths.iter().cloned().collect();
        pending_write_back_paths.sort();
        let mut remote0_paths = Vec::new();
        let mut remote0_error = None;

        let mut agents = Vec::with_capacity(self.agents.len());
        for agent in &self.agents {
            let work_paths = sorted_map_keys(self.oracle.files_for(agent.id, MountId::Work));
            let indexed_paths = sorted_map_keys(self.oracle.files_for(agent.id, MountId::Indexed));
            let shared_write_paths = sorted_map_keys(self.oracle.shared_write_files());

            let (remote_indexed_paths, remote_indexed_error) =
                match collect_backend_paths(agent.indexed_backend.as_ref()).await {
                    Ok(paths) => (paths, None),
                    Err(err) => (Vec::new(), Some(err)),
                };

            let sync = if let Some(handle) = &agent.write_back_handle {
                let status = handle.status().await;
                let (wal_present, wal_pending, wal_processing, wal_failed, wal_unapplied) =
                    if let Some(wal) = handle.wal() {
                        match wal.outbox_stats() {
                            Ok(stats) => (
                                true,
                                stats.pending,
                                stats.processing,
                                stats.failed,
                                stats.wal_unapplied,
                            ),
                            Err(_) => (true, 0, 0, 0, 0),
                        }
                    } else {
                        (false, 0, 0, 0, 0)
                    };

                let (wal_paths_pending, wal_paths_processing, wal_paths_failed) = if let Some(wal) =
                    handle.wal()
                {
                    if let Ok(entries) = wal.outbox_entries() {
                        let mut pending = Vec::new();
                        let mut processing = Vec::new();
                        let mut failed = Vec::new();
                        for entry in entries {
                            match entry.status {
                                openfs_remote::wal::OutboxStatus::Pending => pending.push(entry.path),
                                openfs_remote::wal::OutboxStatus::Processing => {
                                    processing.push(entry.path)
                                }
                                openfs_remote::wal::OutboxStatus::Failed => failed.push(entry.path),
                            }
                        }
                        (
                            dedupe_sorted_paths(pending),
                            dedupe_sorted_paths(processing),
                            dedupe_sorted_paths(failed),
                        )
                    } else {
                        (Vec::new(), Vec::new(), Vec::new())
                    }
                } else {
                    (Vec::new(), Vec::new(), Vec::new())
                };

                Some(SimSyncFrame {
                    mode: format!("{:?}", status.sync_mode),
                    pending: status.sync.pending,
                    synced: status.sync.synced,
                    failed: status.sync.failed,
                    retries: status.sync.retries,
                    cache_entries: status.cache.entries,
                    cache_hits: status.cache.hits,
                    cache_misses: status.cache.misses,
                    wal_present,
                    wal_pending,
                    wal_processing,
                    wal_failed,
                    wal_unapplied,
                    wal_paths_pending,
                    wal_paths_processing,
                    wal_paths_failed,
                })
            } else {
                None
            };

            agents.push(SimAgentFrame {
                agent_id: agent.id,
                work_paths,
                indexed_paths,
                shared_write_paths,
                remote_indexed_paths,
                remote_indexed_error,
                sync,
            });

            if agent.id == 2 {
                remote0_paths = agents
                    .last()
                    .map(|a| a.remote_indexed_paths.clone())
                    .unwrap_or_default();
                remote0_error = agents.last().and_then(|a| a.remote_indexed_error.clone());
            }
        }

        let frame = self.history.len();
        self.history.push(SimStateFrame {
            frame,
            step,
            label: label.map(ToString::to_string),
            violations: self.violations.len(),
            pending_write_back_paths,
            remote0_paths,
            remote0_error,
            agents,
        });
    }

    /// Track paths that are pending write-back (agent 1's indexed mount).
    fn track_write_back(&mut self, agent_id: usize, op: &Op, expected: &Expected) {
        if matches!(op, Op::FlushWriteBack) {
            // Flush clears pending state (may be re-populated in concurrent step)
            self.pending_write_back_paths.clear();
            return;
        }

        for path in Self::pending_paths_for_op(agent_id, op, expected) {
            self.pending_write_back_paths.insert(path);
        }
    }

    fn pending_paths_for_op(agent_id: usize, op: &Op, expected: &Expected) -> Vec<String> {
        if agent_id != 1 || !matches!(expected, Expected::Ok) {
            return Vec::new();
        }

        match op {
            Op::Write {
                mount: MountId::Indexed,
                path,
                ..
            }
            | Op::Append {
                mount: MountId::Indexed,
                path,
                ..
            }
            | Op::Delete {
                mount: MountId::Indexed,
                path,
            } => vec![path.clone()],
            Op::Rename {
                mount: MountId::Indexed,
                from,
                to,
            } => vec![from.clone(), to.clone()],
            _ => Vec::new(),
        }
    }

    fn relax_shared_indexed_outcome(&self, agent_id: usize, op: &Op) -> bool {
        if !self.indexed_shared_12 || !(1..=2).contains(&agent_id) {
            return false;
        }

        matches!(
            op,
            Op::Read {
                mount: MountId::Indexed,
                ..
            } | Op::List {
                mount: MountId::Indexed,
                ..
            } | Op::Stat {
                mount: MountId::Indexed,
                ..
            } | Op::Exists {
                mount: MountId::Indexed,
                ..
            } | Op::IndexFile { .. }
        )
    }

    /// Execute a concurrent step: generate one op per agent and run them simultaneously.
    pub async fn step_concurrent(&mut self) -> Vec<Violation> {
        // Generate one op per agent
        let op0 = generate(&mut self.rng, &self.agent_states[0], self.step);
        let op1 = generate(&mut self.rng, &self.agent_states[1], self.step);

        self.step_concurrent_ops(op0, op1).await
    }

    /// Execute a concurrent step with explicit ops (useful for tests).
    pub async fn step_concurrent_with(&mut self, op0: Op, op1: Op) -> Vec<Violation> {
        self.step_concurrent_ops(op0, op1).await
    }

    async fn step_concurrent_ops(&mut self, op0: Op, op1: Op) -> Vec<Violation> {
        let mut new_violations = Vec::new();
        let flush_in_step = matches!(op0, Op::FlushWriteBack) || matches!(op1, Op::FlushWriteBack);
        let mut pending_from_step: HashSet<String> = HashSet::new();

        // Execute both ops concurrently
        let (actual0, actual1) = {
            let agents = &self.agents;
            let pipeline = &self.pipeline;
            let fut0 = Self::execute_static(agents, pipeline, 0, &op0);
            let fut1 = Self::execute_static(agents, pipeline, 1, &op1);
            tokio::join!(fut0, fut1)
        };

        // Determine whether faults were injected (error path)
        let fault0 = matches!(&actual0, Outcome::Error(msg) if is_injected_fault(msg));
        let fault1 = matches!(&actual1, Outcome::Error(msg) if is_injected_fault(msg));

        // Shared-write conflict detection (only if both ops touch same keys)
        let shared_conflict = !fault0
            && !fault1
            && op_is_mutating_shared_write(&op0)
            && op_is_mutating_shared_write(&op1)
            && shared_write_intersects(&op0, &op1);

        if shared_conflict {
            let base = self.oracle.shared_write_files().clone();
            let touched = shared_write_touched_keys(&op0, &op1);
            let actual_map =
                read_shared_write_map(self.agents[0].shared_write.as_ref(), &touched).await;

            let order_a = simulate_shared_write_order(&base, &op0, &op1, true);
            let order_b = simulate_shared_write_order(&base, &op0, &op1, false);

            let map_match_a = shared_map_matches(&actual_map, &order_a.final_map, &touched);
            let map_match_b = shared_map_matches(&actual_map, &order_b.final_map, &touched);

            let outcome_match_a = outcome_matches_simple(&order_a.expected0, &actual0)
                && outcome_matches_simple(&order_a.expected1, &actual1);
            let outcome_match_b = outcome_matches_simple(&order_b.expected0, &actual0)
                && outcome_matches_simple(&order_b.expected1, &actual1);

            let chosen = if map_match_a && map_match_b {
                if outcome_match_a && !outcome_match_b {
                    Some((&order_a, "A"))
                } else if outcome_match_b && !outcome_match_a {
                    Some((&order_b, "B"))
                } else {
                    Some((&order_a, "A"))
                }
            } else if map_match_a {
                Some((&order_a, "A"))
            } else if map_match_b {
                Some((&order_b, "B"))
            } else {
                None
            };

            if let Some((order, label)) = chosen {
                // Commit in the chosen order so oracle matches the observed state
                let (first, second) = if order.first_is_op0 {
                    ((0, &op0, &order.expected0), (1, &op1, &order.expected1))
                } else {
                    ((1, &op1, &order.expected1), (0, &op0, &order.expected0))
                };

                self.oracle.commit(first.0, first.1);
                self.update_agent_state(first.0, first.1, first.2);

                self.oracle.commit(second.0, second.1);
                self.update_agent_state(second.0, second.1, second.2);

                // Check outcomes and record mismatches (if any)
                if !outcome_matches_simple(&order.expected0, &actual0) {
                    if let Some(v) = check_outcome(self.step, 0, &op0, &order.expected0, &actual0) {
                        self.violations.push(v.clone());
                        new_violations.push(v);
                    }
                }
                if !outcome_matches_simple(&order.expected1, &actual1) {
                    if let Some(v) = check_outcome(self.step, 1, &op1, &order.expected1, &actual1) {
                        self.violations.push(v.clone());
                        new_violations.push(v);
                    }
                }
                let note = format!("shared-conflict-order-{}", label);
                self.push_trace(
                    self.step,
                    0,
                    &op0,
                    &order.expected0,
                    &actual0,
                    Some(note.as_str()),
                );
                self.push_trace(
                    self.step,
                    1,
                    &op1,
                    &order.expected1,
                    &actual1,
                    Some(note.as_str()),
                );
            } else {
                let v = Violation {
                    step: self.step,
                    agent_id: 0,
                    invariant: "concurrent-linearizability".to_string(),
                    details: "Shared write conflict did not match any serial order".to_string(),
                };
                self.violations.push(v.clone());
                new_violations.push(v);

                let expected0 = self.oracle.predict(0, &op0);
                let expected1 = self.oracle.predict(1, &op1);
                self.push_trace(
                    self.step,
                    0,
                    &op0,
                    &expected0,
                    &actual0,
                    Some("shared-conflict-unresolved"),
                );
                self.push_trace(
                    self.step,
                    1,
                    &op1,
                    &expected1,
                    &actual1,
                    Some("shared-conflict-unresolved"),
                );
            }
        } else {
            let race = shared_read_write_race(&op0, &op1);
            let base_shared = if race {
                Some(self.oracle.shared_write_files().clone())
            } else {
                None
            };
            let write_op = if race {
                if op_is_mutating_shared_write(&op0) {
                    Some(&op0)
                } else if op_is_mutating_shared_write(&op1) {
                    Some(&op1)
                } else {
                    None
                }
            } else {
                None
            };

            for (agent_id, op, actual, was_fault) in
                [(0, &op0, &actual0, fault0), (1, &op1, &actual1, fault1)]
            {
                let expected = self.oracle.predict(agent_id, op);

                if was_fault {
                    self.push_trace(
                        self.step,
                        agent_id,
                        op,
                        &expected,
                        actual,
                        Some("fault-injected"),
                    );
                    continue;
                }

                // Write-back mismatch check (agent 1 indexed ops)
                let is_write_back_mismatch = agent_id == 1
                    && is_write_back_affected_op(op)
                    && write_back_op_touches_pending(op, &self.pending_write_back_paths)
                    && matches!(actual, Outcome::NotFound | Outcome::Error(_));

                if is_write_back_mismatch {
                    self.push_trace(
                        self.step,
                        agent_id,
                        op,
                        &expected,
                        actual,
                        Some("write-back-pending-mismatch"),
                    );
                    continue;
                }

                self.oracle.commit(agent_id, op);

                if race && op_is_readlike_shared_write(op) {
                    if let Some(base) = &base_shared {
                        if let Some(write_op) = write_op {
                            let read_ok = shared_readlike_race_ok(base, write_op, op, actual);
                            if !read_ok {
                                if let Some(v) =
                                    check_outcome(self.step, agent_id, op, &expected, actual)
                                {
                                    self.violations.push(v.clone());
                                    new_violations.push(v);
                                }
                            }
                        }
                    }
                } else if !(self.has_faults && matches!(op, Op::Read { .. }))
                    && !self.relax_shared_indexed_outcome(agent_id, op)
                {
                    if let Some(v) = check_outcome(self.step, agent_id, op, &expected, actual) {
                        self.violations.push(v.clone());
                        new_violations.push(v);
                    }
                }

                self.update_agent_state(agent_id, op, &expected);
                for path in Self::pending_paths_for_op(agent_id, op, &expected) {
                    pending_from_step.insert(path);
                }
                self.track_write_back(agent_id, op, &expected);
                self.push_trace(self.step, agent_id, op, &expected, actual, None);
            }
        }

        if flush_in_step {
            self.pending_write_back_paths.clear();
            for path in pending_from_step {
                self.pending_write_back_paths.insert(path);
            }
        }

        // Run invariant checks
        let step_violations = check_step_invariants(
            self.step,
            &self.agents,
            &self.oracle,
            &self.pending_write_back_paths,
            self.has_faults,
        )
        .await;
        self.violations.extend(step_violations.clone());
        new_violations.extend(step_violations);

        self.step += 1;
        self.capture_history_snapshot(self.step, Some("concurrent-step"))
            .await;
        new_violations
    }

    /// Run concurrent batches and return all violations.
    pub async fn run_concurrent(&mut self, batches: usize) -> &[Violation] {
        for _ in 0..batches {
            let _ = self.step_concurrent().await;
        }

        self.shutdown().await;
        self.capture_history_snapshot(self.step, Some("post-shutdown"))
            .await;

        let final_violations = check_final_consistency(&self.agents, &self.oracle).await;
        self.violations.extend(final_violations);
        self.capture_history_snapshot(self.step, Some("final"))
            .await;

        &self.violations
    }

    /// Static version of execute that takes references, usable in tokio::join!
    async fn execute_static(
        agents: &[AgentVm],
        pipeline: &IndexingPipeline,
        agent_id: usize,
        op: &Op,
    ) -> Outcome {
        let agent = &agents[agent_id];

        match op {
            Op::Write {
                mount,
                path,
                content,
            } => {
                let full_path = format!("{}/{}", mount.prefix(agent_id), path);
                match execute_write(&agent.router, &full_path, content).await {
                    Ok(()) => Outcome::Ok,
                    Err(e) => classify_error(e),
                }
            }

            Op::Read { mount, path } => {
                let full_path = format!("{}/{}", mount.prefix(agent_id), path);
                match agent.router.resolve(&full_path) {
                    Ok((backend, relative, _)) => match backend.read(&relative).await {
                        Ok(data) => Outcome::ReadOk(data),
                        Err(e) => classify_backend_error(e),
                    },
                    Err(e) => classify_error(e),
                }
            }

            Op::Append {
                mount,
                path,
                content,
            } => {
                let full_path = format!("{}/{}", mount.prefix(agent_id), path);
                match execute_append(&agent.router, &full_path, content).await {
                    Ok(()) => Outcome::Ok,
                    Err(e) => classify_error(e),
                }
            }

            Op::Delete { mount, path } => {
                let full_path = format!("{}/{}", mount.prefix(agent_id), path);
                match execute_delete(&agent.router, &full_path).await {
                    Ok(()) => Outcome::Ok,
                    Err(e) => classify_error(e),
                }
            }

            Op::List { mount, path } => {
                let full_path = if path.is_empty() {
                    mount.prefix(agent_id).to_string()
                } else {
                    format!("{}/{}", mount.prefix(agent_id), path)
                };
                match agent.router.resolve(&full_path) {
                    Ok((backend, relative, _)) => match backend.list(&relative).await {
                        Ok(entries) => {
                            let summaries = entries.iter().map(EntrySummary::from_entry).collect();
                            Outcome::ListOk(summaries)
                        }
                        Err(e) => classify_backend_error(e),
                    },
                    Err(e) => classify_error(e),
                }
            }

            Op::Stat { mount, path } => {
                let full_path = format!("{}/{}", mount.prefix(agent_id), path);
                match agent.router.resolve(&full_path) {
                    Ok((backend, relative, _)) => match backend.stat(&relative).await {
                        Ok(entry) => Outcome::StatOk(EntrySummary::from_entry(&entry)),
                        Err(e) => classify_backend_error(e),
                    },
                    Err(e) => classify_error(e),
                }
            }

            Op::Exists { mount, path } => {
                let full_path = format!("{}/{}", mount.prefix(agent_id), path);
                match agent.router.resolve(&full_path) {
                    Ok((backend, relative, _)) => match backend.exists(&relative).await {
                        Ok(exists) => Outcome::ExistsOk(exists),
                        Err(e) => classify_backend_error(e),
                    },
                    Err(e) => classify_error(e),
                }
            }

            Op::Rename { mount, from, to } => {
                let from_full = format!("{}/{}", mount.prefix(agent_id), from);
                let to_full = format!("{}/{}", mount.prefix(agent_id), to);
                match execute_rename(&agent.router, &from_full, &to_full).await {
                    Ok(()) => Outcome::Ok,
                    Err(e) => classify_error(e),
                }
            }

            Op::IndexFile { path } => {
                let full_path = format!("{}/{}", MountId::Indexed.prefix(agent_id), path);
                match agent.router.resolve(&full_path) {
                    Ok((backend, relative, _)) => match backend.read(&relative).await {
                        Ok(content) => match pipeline.index_file(path, &content).await {
                            Ok(_) => Outcome::IndexOk,
                            Err(_) => Outcome::Error("indexing_failed".to_string()),
                        },
                        Err(BackendError::NotFound(_)) => Outcome::NotFound,
                        Err(e) => Outcome::Error(e.to_string()),
                    },
                    Err(e) => classify_error(e),
                }
            }

            Op::SearchChroma { query } => match pipeline.embed_query(query).await {
                Ok(embedding) => match agent.chroma.query_by_embedding(embedding, 5).await {
                    Ok(_) => Outcome::SearchOk,
                    Err(e) => Outcome::Error(e.to_string()),
                },
                Err(_) => Outcome::SearchOk,
            },

            Op::FlushWriteBack => {
                // Advance tokio time past the flush interval to trigger background flush
                tokio::task::yield_now().await;
                tokio::time::advance(Duration::from_secs(2)).await;
                tokio::task::yield_now().await;
                Outcome::FlushOk
            }
        }
    }

    /// Execute an operation against the real OpenFS system.
    async fn execute(&self, agent_id: usize, op: &Op) -> Outcome {
        Self::execute_static(&self.agents, &self.pipeline, agent_id, op).await
    }

    /// Update agent operation state after a successful op.
    fn update_agent_state(&mut self, agent_id: usize, op: &Op, expected: &Expected) {
        match op {
            Op::Write { mount, path, .. } => {
                if matches!(expected, Expected::Ok) {
                    {
                        let state = &mut self.agent_states[agent_id];
                        state.add_file(*mount, path.clone());
                        state.file_counter += 1;
                    }

                    // Shared-write is visible to all agents.
                    if *mount == MountId::SharedWrite {
                        for (other_id, state) in self.agent_states.iter_mut().enumerate() {
                            if other_id != agent_id {
                                state.add_file(MountId::SharedWrite, path.clone());
                            }
                        }
                    }
                } else {
                    // Still increment counter to avoid reusing names
                    self.agent_states[agent_id].file_counter += 1;
                }
            }
            Op::Delete { mount, path, .. } => {
                if matches!(expected, Expected::Ok) {
                    {
                        let state = &mut self.agent_states[agent_id];
                        state.remove_file(*mount, path);
                    }

                    if *mount == MountId::SharedWrite {
                        for (other_id, state) in self.agent_states.iter_mut().enumerate() {
                            if other_id != agent_id {
                                state.remove_file(MountId::SharedWrite, path);
                            }
                        }
                    }
                }
            }
            Op::Append { mount, path, .. } => {
                if matches!(expected, Expected::Ok) {
                    {
                        let state = &mut self.agent_states[agent_id];
                        state.add_file(*mount, path.clone());
                    }

                    if *mount == MountId::SharedWrite {
                        for (other_id, state) in self.agent_states.iter_mut().enumerate() {
                            if other_id != agent_id {
                                state.add_file(MountId::SharedWrite, path.clone());
                            }
                        }
                    }
                }
            }
            Op::Rename { mount, from, to } => {
                if matches!(expected, Expected::Ok) {
                    {
                        let state = &mut self.agent_states[agent_id];
                        state.remove_file(*mount, from);
                        state.add_file(*mount, to.clone());
                    }

                    if *mount == MountId::SharedWrite {
                        for (other_id, state) in self.agent_states.iter_mut().enumerate() {
                            if other_id != agent_id {
                                state.remove_file(MountId::SharedWrite, from);
                                state.add_file(MountId::SharedWrite, to.clone());
                            }
                        }
                    }
                }

                // Avoid reusing rename targets across attempts
                self.agent_states[agent_id].file_counter += 1;
            }
            Op::IndexFile { path } => {
                if matches!(expected, Expected::IndexOk) {
                    self.agent_states[agent_id].indexed_files.push(path.clone());
                }
            }
            _ => {}
        }
    }

    /// Strong post-conditions to validate immediate consistency.
    async fn verify_post_conditions(
        &self,
        step: usize,
        agent_id: usize,
        op: &Op,
        expected: &Expected,
    ) -> Option<Violation> {
        let agent = &self.agents[agent_id];

        match op {
            Op::Write { mount, path, .. } | Op::Append { mount, path, .. } => {
                if !matches!(expected, Expected::Ok) {
                    return None;
                }
                let expected_map = self.oracle.files_for(agent_id, *mount);
                let expected_content = match expected_map.get(path) {
                    Some(c) => c,
                    None => return None,
                };
                let full_path = format!("{}/{}", mount.prefix(agent_id), path);
                match agent.router.resolve(&full_path) {
                    Ok((backend, relative, _)) => match backend.read(&relative).await {
                        Ok(actual) => {
                            if actual != *expected_content {
                                Some(Violation {
                                    step,
                                    agent_id,
                                    invariant: "read-after-write".to_string(),
                                    details: format!(
                                        "Read-after-write mismatch for '{}': expected {} bytes, got {} bytes",
                                        full_path,
                                        expected_content.len(),
                                        actual.len()
                                    ),
                                })
                            } else {
                                None
                            }
                        }
                        Err(e) => Some(Violation {
                            step,
                            agent_id,
                            invariant: "read-after-write".to_string(),
                            details: format!("Read-after-write failed for '{}': {}", full_path, e),
                        }),
                    },
                    Err(e) => Some(Violation {
                        step,
                        agent_id,
                        invariant: "read-after-write".to_string(),
                        details: format!(
                            "Read-after-write resolve failed for '{}': {}",
                            full_path, e
                        ),
                    }),
                }
            }
            Op::Rename { mount, from, to } => {
                if !matches!(expected, Expected::Ok) {
                    return None;
                }
                let from_full = format!("{}/{}", mount.prefix(agent_id), from);
                let to_full = format!("{}/{}", mount.prefix(agent_id), to);

                // Old path should be gone.
                if let Ok((backend, relative, _)) = agent.router.resolve(&from_full) {
                    if backend.read(&relative).await.is_ok() {
                        return Some(Violation {
                            step,
                            agent_id,
                            invariant: "rename-atomicity".to_string(),
                            details: format!("Rename left old path readable: '{}'", from_full),
                        });
                    }
                }

                let expected_map = self.oracle.files_for(agent_id, *mount);
                let expected_content = match expected_map.get(to) {
                    Some(c) => c,
                    None => return None,
                };
                match agent.router.resolve(&to_full) {
                    Ok((backend, relative, _)) => match backend.read(&relative).await {
                        Ok(actual) => {
                            if actual != *expected_content {
                                Some(Violation {
                                    step,
                                    agent_id,
                                    invariant: "rename-atomicity".to_string(),
                                    details: format!(
                                        "Rename target mismatch for '{}': expected {} bytes, got {} bytes",
                                        to_full,
                                        expected_content.len(),
                                        actual.len()
                                    ),
                                })
                            } else {
                                None
                            }
                        }
                        Err(e) => Some(Violation {
                            step,
                            agent_id,
                            invariant: "rename-atomicity".to_string(),
                            details: format!("Rename target read failed for '{}': {}", to_full, e),
                        }),
                    },
                    Err(e) => Some(Violation {
                        step,
                        agent_id,
                        invariant: "rename-atomicity".to_string(),
                        details: format!("Rename target resolve failed for '{}': {}", to_full, e),
                    }),
                }
            }
            _ => None,
        }
    }
}

fn sorted_map_keys(map: &HashMap<String, Vec<u8>>) -> Vec<String> {
    let mut keys: Vec<String> = map.keys().cloned().collect();
    keys.sort();
    keys
}

fn dedupe_sorted_paths(mut paths: Vec<String>) -> Vec<String> {
    paths.sort();
    paths.dedup();
    paths
}

async fn collect_backend_paths(backend: &dyn Backend) -> Result<Vec<String>, String> {
    let mut files = BTreeSet::new();
    let mut stack = vec![String::new()];
    let mut visited_dirs = BTreeSet::new();

    while let Some(dir) = stack.pop() {
        if !visited_dirs.insert(dir.clone()) {
            continue;
        }

        let mut entries = backend.list(&dir).await.map_err(|e| e.to_string())?;
        entries.sort_by(|a, b| a.path.cmp(&b.path));

        for entry in entries.into_iter().rev() {
            let normalized = entry.path.trim_matches('/').to_string();
            if normalized.is_empty() {
                continue;
            }
            if entry.is_dir {
                stack.push(normalized);
            } else {
                files.insert(normalized);
            }
        }
    }

    Ok(files.into_iter().collect())
}

/// Outcome of executing an operation against the real system.
#[derive(Debug)]
enum Outcome {
    Ok,
    ReadOk(Vec<u8>),
    ReadOnly,
    NotFound,
    ExistsOk(bool),
    ListOk(Vec<EntrySummary>),
    StatOk(EntrySummary),
    IndexOk,
    SearchOk,
    FlushOk,
    Error(String),
}

/// Execute a write via the router (handles read-only check like Vfs does).
async fn execute_write(
    router: &openfs_remote::Router,
    path: &str,
    content: &[u8],
) -> Result<(), VfsError> {
    let (backend, relative, read_only) = router.resolve(path)?;
    if read_only {
        return Err(VfsError::ReadOnly(path.to_string()));
    }
    backend
        .write(&relative, content)
        .await
        .map_err(VfsError::from)
}

async fn execute_append(
    router: &openfs_remote::Router,
    path: &str,
    content: &[u8],
) -> Result<(), VfsError> {
    let (backend, relative, read_only) = router.resolve(path)?;
    if read_only {
        return Err(VfsError::ReadOnly(path.to_string()));
    }
    backend
        .append(&relative, content)
        .await
        .map_err(VfsError::from)
}

async fn execute_delete(router: &openfs_remote::Router, path: &str) -> Result<(), VfsError> {
    let (backend, relative, read_only) = router.resolve(path)?;
    if read_only {
        return Err(VfsError::ReadOnly(path.to_string()));
    }
    backend.delete(&relative).await.map_err(VfsError::from)
}

async fn execute_rename(router: &openfs_remote::Router, from: &str, to: &str) -> Result<(), VfsError> {
    let (backend, relative, read_only) = router.resolve(from)?;
    if read_only {
        return Err(VfsError::ReadOnly(from.to_string()));
    }
    let (_to_backend, relative_to, _to_ro) = router.resolve(to)?;
    backend
        .rename(&relative, &relative_to)
        .await
        .map_err(VfsError::from)
}

fn classify_error(e: VfsError) -> Outcome {
    match e {
        VfsError::ReadOnly(_) => Outcome::ReadOnly,
        VfsError::NotFound(_) => Outcome::NotFound,
        _ => Outcome::Error(e.to_string()),
    }
}

fn classify_backend_error(e: BackendError) -> Outcome {
    match e {
        BackendError::NotFound(_) => Outcome::NotFound,
        BackendError::PermissionDenied(_) => Outcome::ReadOnly,
        other => Outcome::Error(other.to_string()),
    }
}

/// Compare expected vs actual outcome and return a violation if they don't match.
fn check_outcome(
    step: usize,
    agent_id: usize,
    op: &Op,
    expected: &Expected,
    actual: &Outcome,
) -> Option<Violation> {
    let mismatch = match (expected, actual) {
        (Expected::Ok, Outcome::Ok) => false,
        (Expected::ReadOk(exp), Outcome::ReadOk(act)) => exp != act,
        // Shared mount reads: content may differ due to per-agent caching.
        // We only verify the read succeeds.
        (Expected::SharedWriteOk, Outcome::ReadOk(_)) => false,
        (Expected::ReadOnly, Outcome::ReadOnly) => false,
        (Expected::NotFound, Outcome::NotFound) => false,
        // Shared-write reads can be stale due to per-agent caches; allow ReadOk even if
        // oracle no longer has the file.
        (Expected::NotFound, Outcome::ReadOk(_))
            if matches!(
                op,
                Op::Read {
                    mount: MountId::SharedWrite,
                    ..
                }
            ) =>
        {
            false
        }
        (Expected::ExistsOk(exp), Outcome::ExistsOk(act)) => exp != act,
        (Expected::ListOk(exp), Outcome::ListOk(act)) => !entries_match(exp, act),
        (Expected::StatOk(exp), Outcome::StatOk(act)) => exp != act,
        (Expected::IndexOk, Outcome::IndexOk) => false,
        (Expected::SearchOk, Outcome::SearchOk) => false,
        (Expected::FlushOk, Outcome::FlushOk) => false,

        // NotFound from real system when oracle expects Ok is a real problem
        // (could happen with write-back not yet flushed, but we handle that)
        (Expected::Ok, Outcome::Error(e)) => {
            // Write-back: the sync engine not started error is expected if
            // we couldn't start it. Otherwise it's a real failure.
            !e.contains("Sync engine not started") && !e.contains("Sync channel closed")
        }
        (Expected::NotFound, Outcome::Error(e)) => {
            // Some backends return generic errors instead of NotFound
            !e.contains("not found") && !e.contains("NotFound")
        }

        _ => true,
    };

    if mismatch {
        Some(Violation {
            step,
            agent_id,
            invariant: "outcome-match".to_string(),
            details: format!(
                "Op {:?}: expected {:?}, got {:?}",
                op_summary(op),
                expected,
                actual
            ),
        })
    } else {
        None
    }
}

fn op_summary(op: &Op) -> String {
    match op {
        Op::Write { mount, path, .. } => format!("Write({:?}, {})", mount, path),
        Op::Read { mount, path } => format!("Read({:?}, {})", mount, path),
        Op::Append { mount, path, .. } => format!("Append({:?}, {})", mount, path),
        Op::Delete { mount, path } => format!("Delete({:?}, {})", mount, path),
        Op::List { mount, path } => format!("List({:?}, {})", mount, path),
        Op::Stat { mount, path } => format!("Stat({:?}, {})", mount, path),
        Op::Exists { mount, path } => format!("Exists({:?}, {})", mount, path),
        Op::Rename { mount, from, to } => format!("Rename({:?}, {} -> {})", mount, from, to),
        Op::IndexFile { path } => format!("IndexFile({})", path),
        Op::SearchChroma { query } => format!("SearchChroma({})", query),
        Op::FlushWriteBack => "FlushWriteBack".to_string(),
    }
}

fn expected_summary(expected: &Expected) -> String {
    match expected {
        Expected::Ok => "Ok".to_string(),
        Expected::ReadOk(data) => format!("ReadOk({} bytes)", data.len()),
        Expected::SharedWriteOk => "SharedWriteOk".to_string(),
        Expected::ReadOnly => "ReadOnly".to_string(),
        Expected::NotFound => "NotFound".to_string(),
        Expected::ExistsOk(exists) => format!("ExistsOk({})", exists),
        Expected::ListOk(entries) => format!("ListOk({} entries)", entries.len()),
        Expected::StatOk(entry) => format!("StatOk(name={}, dir={})", entry.name, entry.is_dir),
        Expected::IndexOk => "IndexOk".to_string(),
        Expected::SearchOk => "SearchOk".to_string(),
        Expected::FlushOk => "FlushOk".to_string(),
    }
}

fn outcome_summary(actual: &Outcome) -> String {
    match actual {
        Outcome::Ok => "Ok".to_string(),
        Outcome::ReadOk(data) => format!("ReadOk({} bytes)", data.len()),
        Outcome::ReadOnly => "ReadOnly".to_string(),
        Outcome::NotFound => "NotFound".to_string(),
        Outcome::ExistsOk(exists) => format!("ExistsOk({})", exists),
        Outcome::ListOk(entries) => format!("ListOk({} entries)", entries.len()),
        Outcome::StatOk(entry) => format!("StatOk(name={}, dir={})", entry.name, entry.is_dir),
        Outcome::IndexOk => "IndexOk".to_string(),
        Outcome::SearchOk => "SearchOk".to_string(),
        Outcome::FlushOk => "FlushOk".to_string(),
        Outcome::Error(err) => format!("Error({})", err),
    }
}

fn entries_match(expected: &[EntrySummary], actual: &[EntrySummary]) -> bool {
    if expected.len() != actual.len() {
        return false;
    }

    let mut exp = expected.to_vec();
    let mut act = actual.to_vec();
    exp.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });
    act.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });

    exp == act
}

/// Extract the mount targeted by an op, if applicable.
fn op_mount(op: &Op) -> Option<MountId> {
    match op {
        Op::Write { mount, .. }
        | Op::Read { mount, .. }
        | Op::Append { mount, .. }
        | Op::Delete { mount, .. }
        | Op::List { mount, .. }
        | Op::Stat { mount, .. }
        | Op::Exists { mount, .. }
        | Op::Rename { mount, .. } => Some(*mount),
        Op::IndexFile { .. } => Some(MountId::Indexed),
        Op::SearchChroma { .. } => None,
        Op::FlushWriteBack => None,
    }
}

/// Check if an op is a mutating operation.
fn op_is_mutating(op: &Op) -> bool {
    matches!(
        op,
        Op::Write { .. } | Op::Append { .. } | Op::Delete { .. } | Op::Rename { .. }
    )
}

fn op_is_mutating_shared_write(op: &Op) -> bool {
    op_is_mutating(op) && op_mount(op) == Some(MountId::SharedWrite)
}

fn op_is_readlike_shared_write(op: &Op) -> bool {
    matches!(
        op,
        Op::Read {
            mount: MountId::SharedWrite,
            ..
        } | Op::Exists {
            mount: MountId::SharedWrite,
            ..
        } | Op::Stat {
            mount: MountId::SharedWrite,
            ..
        }
    )
}

/// Check if an op targets agent 1's indexed mount AND uses an operation that
/// goes to the inner backend directly (bypassing cache), which means write-back
/// data won't be visible.
fn is_write_back_affected_op(op: &Op) -> bool {
    match op {
        Op::Stat {
            mount: MountId::Indexed,
            ..
        }
        | Op::List {
            mount: MountId::Indexed,
            ..
        }
        | Op::Rename {
            mount: MountId::Indexed,
            ..
        } => true,
        // IndexFile reads via router (cache-aware), but then reads the content
        // for indexing. If the file was renamed from a write-back-only path, it
        // may not exist in the inner backend for the rename to find.
        Op::IndexFile { .. } => true,
        _ => false,
    }
}

fn write_back_op_touches_pending(op: &Op, pending: &HashSet<String>) -> bool {
    if pending.is_empty() {
        return false;
    }

    match op {
        Op::Stat {
            mount: MountId::Indexed,
            path,
        }
        | Op::List {
            mount: MountId::Indexed,
            path,
        } => pending_affects_path(path, pending),
        Op::Rename {
            mount: MountId::Indexed,
            from,
            to,
        } => pending_affects_path(from, pending) || pending_affects_path(to, pending),
        Op::IndexFile { path } => pending.contains(path),
        _ => false,
    }
}

fn pending_affects_path(path: &str, pending: &HashSet<String>) -> bool {
    if pending.is_empty() {
        return false;
    }
    let norm = path.trim_matches('/');
    if norm.is_empty() {
        return true;
    }
    let prefix = format!("{}/", norm);
    pending.iter().any(|p| p == norm || p.starts_with(&prefix))
}

#[derive(Debug)]
struct SharedWriteOrder {
    expected0: Expected,
    expected1: Expected,
    final_map: HashMap<String, Vec<u8>>,
    first_is_op0: bool,
}

fn shared_write_intersects(op0: &Op, op1: &Op) -> bool {
    let keys0: HashSet<String> = shared_write_keys(op0).into_iter().collect();
    let keys1: HashSet<String> = shared_write_keys(op1).into_iter().collect();
    !keys0.is_disjoint(&keys1)
}

fn shared_write_touched_keys(op0: &Op, op1: &Op) -> Vec<String> {
    let mut keys: HashSet<String> = HashSet::new();
    for k in shared_write_keys(op0) {
        keys.insert(k);
    }
    for k in shared_write_keys(op1) {
        keys.insert(k);
    }
    let mut out: Vec<String> = keys.into_iter().collect();
    out.sort();
    out
}

fn shared_write_keys(op: &Op) -> Vec<String> {
    match op {
        Op::Write {
            mount: MountId::SharedWrite,
            path,
            ..
        }
        | Op::Append {
            mount: MountId::SharedWrite,
            path,
            ..
        }
        | Op::Delete {
            mount: MountId::SharedWrite,
            path,
        } => vec![path.clone()],
        Op::Rename {
            mount: MountId::SharedWrite,
            from,
            to,
        } => vec![from.clone(), to.clone()],
        _ => Vec::new(),
    }
}

fn predict_shared_write(map: &HashMap<String, Vec<u8>>, op: &Op) -> Expected {
    match op {
        Op::Write {
            mount: MountId::SharedWrite,
            ..
        }
        | Op::Append {
            mount: MountId::SharedWrite,
            ..
        } => Expected::Ok,
        Op::Delete {
            mount: MountId::SharedWrite,
            path,
        } => {
            if map.contains_key(path) {
                Expected::Ok
            } else {
                Expected::NotFound
            }
        }
        Op::Rename {
            mount: MountId::SharedWrite,
            from,
            ..
        } => {
            if map.contains_key(from) {
                Expected::Ok
            } else {
                Expected::NotFound
            }
        }
        _ => Expected::Ok,
    }
}

fn apply_shared_write_op(map: &mut HashMap<String, Vec<u8>>, op: &Op) {
    match op {
        Op::Write {
            mount: MountId::SharedWrite,
            path,
            content,
        } => {
            map.insert(path.clone(), content.clone());
        }
        Op::Append {
            mount: MountId::SharedWrite,
            path,
            content,
        } => {
            let entry = map.entry(path.clone()).or_default();
            entry.extend_from_slice(content);
        }
        Op::Delete {
            mount: MountId::SharedWrite,
            path,
        } => {
            map.remove(path);
        }
        Op::Rename {
            mount: MountId::SharedWrite,
            from,
            to,
        } => {
            if let Some(content) = map.remove(from) {
                map.insert(to.clone(), content);
            }
        }
        _ => {}
    }
}

fn simulate_shared_write_order(
    base: &HashMap<String, Vec<u8>>,
    op0: &Op,
    op1: &Op,
    first_is_op0: bool,
) -> SharedWriteOrder {
    let mut map = base.clone();
    let (first, second) = if first_is_op0 { (op0, op1) } else { (op1, op0) };

    let expected_first = predict_shared_write(&map, first);
    apply_shared_write_op(&mut map, first);

    let expected_second = predict_shared_write(&map, second);
    apply_shared_write_op(&mut map, second);

    let (expected0, expected1) = if first_is_op0 {
        (expected_first, expected_second)
    } else {
        (expected_second, expected_first)
    };

    SharedWriteOrder {
        expected0,
        expected1,
        final_map: map,
        first_is_op0,
    }
}

async fn read_shared_write_map(
    backend: &dyn Backend,
    keys: &[String],
) -> HashMap<String, Option<Vec<u8>>> {
    let mut out = HashMap::new();
    for key in keys {
        let value = match backend.read(key).await {
            Ok(data) => Some(data),
            Err(BackendError::NotFound(_)) => None,
            Err(_) => None,
        };
        out.insert(key.clone(), value);
    }
    out
}

fn shared_map_matches(
    actual: &HashMap<String, Option<Vec<u8>>>,
    expected: &HashMap<String, Vec<u8>>,
    keys: &[String],
) -> bool {
    for key in keys {
        let actual_val = actual.get(key).and_then(|v| v.as_ref());
        let expected_val = expected.get(key);
        match (expected_val, actual_val) {
            (Some(exp), Some(act)) => {
                if exp != act {
                    return false;
                }
            }
            (None, None) => {}
            (None, Some(_)) => return false,
            (Some(_), None) => return false,
        }
    }
    true
}

fn outcome_matches_simple(expected: &Expected, actual: &Outcome) -> bool {
    match expected {
        Expected::Ok => matches!(actual, Outcome::Ok),
        Expected::NotFound => matches!(actual, Outcome::NotFound),
        Expected::ReadOnly => matches!(actual, Outcome::ReadOnly),
        _ => false,
    }
}

fn shared_write_read_path(op: &Op) -> Option<&str> {
    match op {
        Op::Read {
            mount: MountId::SharedWrite,
            path,
        }
        | Op::Exists {
            mount: MountId::SharedWrite,
            path,
        }
        | Op::Stat {
            mount: MountId::SharedWrite,
            path,
        } => Some(path),
        _ => None,
    }
}

fn shared_read_write_race(op0: &Op, op1: &Op) -> bool {
    if let Some(path) = shared_write_read_path(op0) {
        if op_is_mutating_shared_write(op1) && shared_write_keys(op1).iter().any(|k| k == path) {
            return true;
        }
    }
    if let Some(path) = shared_write_read_path(op1) {
        if op_is_mutating_shared_write(op0) && shared_write_keys(op0).iter().any(|k| k == path) {
            return true;
        }
    }
    false
}

fn shared_read_race_ok(
    base: &HashMap<String, Vec<u8>>,
    write_op: &Op,
    read_op: &Op,
    actual: &Outcome,
) -> bool {
    let read_path = match read_op {
        Op::Read {
            mount: MountId::SharedWrite,
            path,
        } => path,
        _ => return true,
    };
    let pre = base.get(read_path).cloned();
    let mut post_map = base.clone();
    apply_shared_write_op(&mut post_map, write_op);
    let post = post_map.get(read_path).cloned();

    match actual {
        Outcome::ReadOk(data) => pre.as_deref() == Some(data) || post.as_deref() == Some(data),
        Outcome::NotFound => pre.is_none() || post.is_none(),
        _ => false,
    }
}

fn shared_exists_path(map: &HashMap<String, Vec<u8>>, path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    if map.contains_key(path) {
        return true;
    }
    let prefix = format!("{}/", path);
    map.keys().any(|k| k.starts_with(&prefix))
}

fn shared_stat_entry(map: &HashMap<String, Vec<u8>>, path: &str) -> Option<EntrySummary> {
    if path.is_empty() {
        return None;
    }
    if let Some(content) = map.get(path) {
        let name = path.rsplit('/').next().unwrap_or(path);
        return Some(EntrySummary {
            name: name.to_string(),
            is_dir: false,
            size: Some(content.len() as u64),
        });
    }
    let prefix = format!("{}/", path);
    if map.keys().any(|k| k.starts_with(&prefix)) {
        let name = path.rsplit('/').next().unwrap_or(path);
        return Some(EntrySummary {
            name: name.to_string(),
            is_dir: true,
            size: None,
        });
    }
    None
}

fn shared_readlike_race_ok(
    base: &HashMap<String, Vec<u8>>,
    write_op: &Op,
    read_op: &Op,
    actual: &Outcome,
) -> bool {
    match read_op {
        Op::Read {
            mount: MountId::SharedWrite,
            ..
        } => shared_read_race_ok(base, write_op, read_op, actual),
        Op::Exists {
            mount: MountId::SharedWrite,
            path,
        } => {
            let pre = shared_exists_path(base, path);
            let mut post_map = base.clone();
            apply_shared_write_op(&mut post_map, write_op);
            let post = shared_exists_path(&post_map, path);
            match actual {
                Outcome::ExistsOk(value) => *value == pre || *value == post,
                _ => false,
            }
        }
        Op::Stat {
            mount: MountId::SharedWrite,
            path,
        } => {
            let pre = shared_stat_entry(base, path);
            let mut post_map = base.clone();
            apply_shared_write_op(&mut post_map, write_op);
            let post = shared_stat_entry(&post_map, path);
            match actual {
                Outcome::StatOk(entry) => {
                    pre.as_ref() == Some(entry) || post.as_ref() == Some(entry)
                }
                Outcome::NotFound => pre.is_none() || post.is_none(),
                _ => false,
            }
        }
        _ => true,
    }
}
