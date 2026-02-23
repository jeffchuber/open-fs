use std::time::{SystemTime, UNIX_EPOCH};

use openfs_core::Backend;
use openfs_sim::debug_ui;
use openfs_sim::ops::{MountId, Op};
use openfs_sim::Sim;

#[tokio::test(start_paused = true)]
async fn debug_ui_snapshot_and_bundle_are_generated() {
    let mut sim = Sim::new_with_config(42, None, true).await;
    let _ = sim.run_mixed(24, 0.4).await;

    let snapshot = debug_ui::snapshot_json(&sim).await;
    assert_eq!(snapshot["meta"]["agent_count"], 2);
    assert!(snapshot["trace"]
        .as_array()
        .is_some_and(|rows| !rows.is_empty()));
    assert!(snapshot["agents"]
        .as_array()
        .is_some_and(|agents| agents.len() == 2));
    assert!(snapshot["history"]
        .as_array()
        .is_some_and(|frames| frames.len() > 5));

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    let out_dir = std::env::temp_dir().join(format!("openfs-sim-debug-ui-test-{}", nonce));

    let (html_path, json_path) = debug_ui::write_bundle(&sim, &out_dir)
        .await
        .expect("write debug bundle");

    assert!(html_path.exists(), "html bundle missing");
    assert!(json_path.exists(), "json bundle missing");

    let html = std::fs::read_to_string(&html_path).expect("read html output");
    assert!(html.contains("OpenFS Sim Debug UI"));
    assert!(html.contains("replay-slider"));
    assert!(html.contains("Play"));

    let json = std::fs::read_to_string(&json_path).expect("read json output");
    assert!(json.contains("pending_write_back_paths"));
    assert!(json.contains("\"history\""));

    let _ = std::fs::remove_dir_all(out_dir);
}

#[tokio::test(start_paused = true)]
async fn debug_ui_snapshot_includes_remote0_when_remote_client_enabled() {
    let mut sim = Sim::new_with_remote_client(7, None, true).await;

    let _ = sim
        .step_with(
            2,
            openfs_sim::ops::Op::Write {
                mount: openfs_sim::ops::MountId::Indexed,
                path: "remote0_test.txt".to_string(),
                content: b"remote0 payload".to_vec(),
            },
        )
        .await;
    let _ = sim.run_mixed(12, 0.3).await;

    let snapshot = debug_ui::snapshot_json(&sim).await;
    assert_eq!(snapshot["meta"]["agent_count"], 3);
    assert!(snapshot["remote0"]["files"]
        .as_array()
        .is_some_and(|files| !files.is_empty()));
    assert!(snapshot["history"]
        .as_array()
        .is_some_and(|frames| frames.iter().any(|frame| frame["remote0_paths"].is_array())));
}

#[tokio::test(start_paused = true)]
async fn remote_topology_shares_indexed_backend_between_client1_and_client2() {
    let mut sim = Sim::new_with_remote_client(99, None, true).await;
    let path = "shared_sync_probe.txt".to_string();
    let initial = b"v1".to_vec();
    let updated = b"v2".to_vec();

    let _ = sim
        .step_with(
            2,
            Op::Write {
                mount: MountId::Indexed,
                path: path.clone(),
                content: initial.clone(),
            },
        )
        .await;

    let full_path_a1 = format!("/a1/indexed/{}", path);
    let (a1_backend, a1_relative, _) = sim.agents[1]
        .router
        .resolve(&full_path_a1)
        .expect("resolve a1 indexed path");
    let seen_by_a1 = a1_backend
        .read(&a1_relative)
        .await
        .expect("client1 should see client2 indexed write");
    assert_eq!(seen_by_a1, initial);

    let _ = sim
        .step_with(
            1,
            Op::Write {
                mount: MountId::Indexed,
                path: path.clone(),
                content: updated.clone(),
            },
        )
        .await;
    sim.shutdown().await;

    let seen_by_a2 = sim.agents[2]
        .indexed_backend
        .read(&path)
        .await
        .expect("client2 backing store should see flushed client1 write");
    assert_eq!(seen_by_a2, updated);
}
