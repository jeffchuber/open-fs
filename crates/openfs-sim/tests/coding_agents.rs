use openfs_sim::coding_agents_sim::{CodingAgentSim, CodingSimProfile};
use std::time::{SystemTime, UNIX_EPOCH};

#[tokio::test(start_paused = true)]
async fn coding_agents_sim_runs_many_agents_with_shared_skills_and_memory() {
    let mut sim = CodingAgentSim::new(42, 9).await;
    sim.run_rounds(24).await;

    let summary = sim.summary().await.expect("summary");
    assert_eq!(summary.seed, 42);
    assert_eq!(summary.agent_count, 9);
    assert_eq!(summary.rounds, 24);
    assert_eq!(summary.error_count, 0, "{:?}", sim.errors);
    assert!(summary.trace_entries >= 9 * 24 * 3);
    assert!(summary.code_file_count >= 9);
    assert!(summary.shared_skill_file_count >= 4);
    assert_eq!(summary.agents_with_memory, 9);
    assert!(summary.total_memory_entries >= 9 * 24);
    assert!(summary.code_contributors >= 6);
    assert!(summary.skill_contributors >= 6);
}

#[tokio::test(start_paused = true)]
async fn coding_agents_sim_report_contains_repo_skills_and_agent_memory() {
    let mut sim = CodingAgentSim::new(7, 5).await;
    sim.run_rounds(10).await;

    let report = sim.report_json().await.expect("report");

    assert_eq!(report["summary"]["agent_count"], 5);
    assert_eq!(report["summary"]["errors"], 0);

    let repo_files = report["repo"]["files"]
        .as_array()
        .expect("repo files should be array");
    assert!(repo_files.len() >= 5);

    let skill_files = report["skills"]["files"]
        .as_array()
        .expect("skill files should be array");
    assert!(skill_files.len() >= 4);

    let agents = report["agents"].as_array().expect("agents should be array");
    assert_eq!(agents.len(), 5);
    for agent in agents {
        let memory_files = agent["memory"]["files"]
            .as_array()
            .expect("memory files should be array");
        assert!(
            memory_files
                .iter()
                .any(|file| file["path"] == "memory/notes.md"),
            "expected memory/notes.md for agent report entry: {}",
            agent
        );
    }

    let history = report["history"]
        .as_array()
        .expect("history should be an array");
    assert!(
        history.len() >= 11,
        "history should include initial + one frame per round"
    );
    assert_eq!(history[0]["label"], "initial");
    assert!(history.iter().any(|frame| frame["round_actions"]
        .as_array()
        .is_some_and(|rows| !rows.is_empty())));
}

#[tokio::test(start_paused = true)]
async fn coding_agents_sim_bundle_writes_replay_html_and_data_json() {
    let mut sim = CodingAgentSim::new(3, 6).await;
    sim.run_rounds(12).await;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    let out_dir = std::env::temp_dir().join(format!("openfs-coding-sim-ui-test-{}", nonce));

    let (html_path, json_path) = openfs_sim::coding_agents_sim::write_bundle(&sim, &out_dir)
        .await
        .expect("write coding agents bundle");

    assert!(html_path.exists(), "html bundle missing");
    assert!(json_path.exists(), "json bundle missing");

    let html = std::fs::read_to_string(&html_path).expect("read html output");
    assert!(html.contains("OpenFS Coding Agent Sim UI"));
    assert!(html.contains("replay-slider"));
    assert!(html.contains("Play"));

    let json = std::fs::read_to_string(&json_path).expect("read json output");
    assert!(json.contains("\"history\""));
    assert!(json.contains("\"round_actions\""));

    let _ = std::fs::remove_dir_all(out_dir);
}

#[tokio::test(start_paused = true)]
async fn coding_agents_sim_cas_demo_surfaces_conflicts_and_retries() {
    let mut sim = CodingAgentSim::new_with_profile(99, 8, CodingSimProfile::CasDemo).await;
    sim.run_rounds(10).await;

    let summary = sim.summary().await.expect("summary");
    assert_eq!(summary.profile, CodingSimProfile::CasDemo);
    assert!(
        summary.cas_conflicts > 0,
        "expected CAS conflicts in cas-demo profile"
    );
    assert!(
        summary.cas_retried_writes > 0,
        "expected CAS retries in cas-demo profile"
    );
    assert!(
        summary.cas_max_retries > 0,
        "expected at least one write to require retry"
    );

    let report = sim.report_json().await.expect("report");
    assert_eq!(report["summary"]["profile"], "cas-demo");
    assert!(
        report["trace"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| {
                row["action"]
                    .as_str()
                    .is_some_and(|action| action.ends_with("_retry"))
            })),
        "expected retry actions in trace"
    );
}
