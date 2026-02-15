use ax_core::{Backend, ChromaStore};
use ax_sim::Sim;
use ax_sim::invariants::check_final_consistency;
use ax_sim::ops::{MountId, Op};
use ax_sim::FaultConfig;
use serde_json::json;

// ─── Existing tests ─────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn sim_seed_42_10_steps() {
    let mut sim = Sim::new(42).await;
    let violations = sim.run(10).await;
    assert!(violations.is_empty(), "{:#?}", violations);
}

#[tokio::test(start_paused = true)]
async fn sim_seed_42_500_steps() {
    let mut sim = Sim::new(42).await;
    let violations = sim.run(500).await;
    assert!(violations.is_empty(), "{:#?}", violations);
}

#[tokio::test(start_paused = true)]
async fn sim_seed_123_1000_steps() {
    let mut sim = Sim::new(123).await;
    let violations = sim.run(1000).await;
    assert!(violations.is_empty(), "{:#?}", violations);
}

#[tokio::test(start_paused = true)]
async fn sim_fuzz_50_seeds() {
    for seed in 0..50 {
        let mut sim = Sim::new(seed).await;
        let violations = sim.run(200).await;
        assert!(violations.is_empty(), "seed {}: {:#?}", seed, violations);
    }
}

#[tokio::test(start_paused = true)]
async fn sim_detects_corruption() {
    let mut sim = Sim::new(7).await;
    let _ = sim.run(50).await;

    // Corrupt backend state without updating oracle.
    sim.agents[0]
        .work_backend
        .write("corrupt.txt", b"oops")
        .await
        .unwrap();

    let violations = check_final_consistency(&sim.agents, &sim.oracle).await;
    assert!(
        !violations.is_empty(),
        "expected violations after corruption"
    );
}

#[tokio::test(start_paused = true)]
async fn sim_scripted_directory_ops() {
    let mut sim = Sim::new(1).await;

    let v = sim
        .step_with(
            0,
            Op::Write {
                mount: MountId::Work,
                path: "dir/one.txt".to_string(),
                content: b"one".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim
        .step_with(
            0,
            Op::Write {
                mount: MountId::Work,
                path: "dir/sub/two.txt".to_string(),
                content: b"two".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim
        .step_with(
            0,
            Op::List {
                mount: MountId::Work,
                path: "dir".to_string(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim
        .step_with(
            0,
            Op::Stat {
                mount: MountId::Work,
                path: "dir".to_string(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim
        .step_with(
            0,
            Op::Stat {
                mount: MountId::Work,
                path: "dir/sub".to_string(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim
        .step_with(
            0,
            Op::List {
                mount: MountId::Work,
                path: "dir/sub".to_string(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim
        .step_with(
            0,
            Op::Exists {
                mount: MountId::Work,
                path: "dir".to_string(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);
}

#[tokio::test(start_paused = true)]
async fn sim_scripted_rename_overwrite() {
    let mut sim = Sim::new(2).await;

    let v = sim
        .step_with(
            0,
            Op::Write {
                mount: MountId::Work,
                path: "a.txt".to_string(),
                content: b"aaa".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim
        .step_with(
            0,
            Op::Write {
                mount: MountId::Work,
                path: "b.txt".to_string(),
                content: b"bbb".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim
        .step_with(
            0,
            Op::Rename {
                mount: MountId::Work,
                from: "a.txt".to_string(),
                to: "b.txt".to_string(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim
        .step_with(
            0,
            Op::Read {
                mount: MountId::Work,
                path: "b.txt".to_string(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);
}

#[tokio::test(start_paused = true)]
async fn sim_scripted_readonly_ops() {
    let mut sim = Sim::new(3).await;

    let v = sim
        .step_with(
            0,
            Op::Write {
                mount: MountId::SharedRead,
                path: "illegal.txt".to_string(),
                content: b"nope".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim
        .step_with(
            0,
            Op::Append {
                mount: MountId::SharedRead,
                path: "seed_0.txt".to_string(),
                content: b"nope".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim
        .step_with(
            0,
            Op::Delete {
                mount: MountId::SharedRead,
                path: "seed_1.txt".to_string(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim
        .step_with(
            0,
            Op::Rename {
                mount: MountId::SharedRead,
                from: "seed_2.txt".to_string(),
                to: "moved.txt".to_string(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);
}

#[tokio::test(start_paused = true)]
async fn sim_detects_shared_read_corruption() {
    let mut sim = Sim::new(11).await;
    let _ = sim.run(10).await;

    // Mutate shared_read backend directly (should be immutable).
    sim.agents[0]
        .shared_read
        .write("seed_0.txt", b"tampered")
        .await
        .unwrap();
    sim.agents[0]
        .shared_read
        .write("nested/evil.txt", b"evil")
        .await
        .unwrap();

    let violations = check_final_consistency(&sim.agents, &sim.oracle).await;
    assert!(
        !violations.is_empty(),
        "expected violations after shared_read corruption"
    );
}

#[tokio::test(start_paused = true)]
async fn sim_detects_shared_write_corruption() {
    let mut sim = Sim::new(12).await;

    let path = "shared/file.txt".to_string();
    let v = sim
        .step_with(
            0,
            Op::Write {
                mount: MountId::SharedWrite,
                path: path.clone(),
                content: b"good".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    // Remove from backend without updating oracle.
    sim.agents[0].shared_write.delete(&path).await.unwrap();

    let violations = check_final_consistency(&sim.agents, &sim.oracle).await;
    assert!(
        !violations.is_empty(),
        "expected violations after shared_write corruption"
    );
}

#[tokio::test(start_paused = true)]
async fn sim_detects_indexed_backend_corruption() {
    let mut sim = Sim::new(13).await;

    let path = "idx/file.txt".to_string();
    let v = sim
        .step_with(
            0,
            Op::Write {
                mount: MountId::Indexed,
                path: path.clone(),
                content: b"good".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    // Corrupt indexed backend content directly.
    sim.agents[0]
        .indexed_backend
        .write(&path, b"bad")
        .await
        .unwrap();

    let violations = check_final_consistency(&sim.agents, &sim.oracle).await;
    assert!(
        !violations.is_empty(),
        "expected violations after indexed backend corruption"
    );
}

#[tokio::test(start_paused = true)]
async fn sim_detects_chroma_corruption() {
    let mut sim = Sim::new(14).await;

    let path = "idx/chroma.txt".to_string();
    let v = sim
        .step_with(
            0,
            Op::Write {
                mount: MountId::Indexed,
                path: path.clone(),
                content: b"content".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim.step_with(0, Op::IndexFile { path: path.clone() }).await;
    assert!(v.is_empty(), "{:#?}", v);

    // Remove indexed docs from chroma without updating oracle.
    let _ = sim
        .agents[0]
        .chroma
        .delete_by_metadata(json!({"source_path": path}))
        .await
        .unwrap();

    let violations = check_final_consistency(&sim.agents, &sim.oracle).await;
    assert!(
        !violations.is_empty(),
        "expected violations after chroma corruption"
    );
}

#[tokio::test(start_paused = true)]
async fn sim_detects_nested_extra_files() {
    let mut sim = Sim::new(15).await;
    let _ = sim.run(10).await;

    // Add a nested file outside oracle tracking.
    sim.agents[0]
        .work_backend
        .write("deep/nested/extra.txt", b"x")
        .await
        .unwrap();

    let violations = check_final_consistency(&sim.agents, &sim.oracle).await;
    assert!(
        !violations.is_empty(),
        "expected violations after nested extra file"
    );
}

// ─── Fault Injection Tests ──────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn sim_fault_injection_10pct_500_steps() {
    let fc = FaultConfig {
        error_rate: 0.10,
        corruption_rate: 0.0,
    };
    let mut sim = Sim::new_with_faults(42, Some(fc)).await;
    let violations = sim.run(500).await;
    assert!(violations.is_empty(), "{:#?}", violations);
}

#[tokio::test(start_paused = true)]
async fn sim_fault_injection_50pct_200_steps() {
    let fc = FaultConfig {
        error_rate: 0.50,
        corruption_rate: 0.0,
    };
    let mut sim = Sim::new_with_faults(99, Some(fc)).await;
    let violations = sim.run(200).await;
    assert!(violations.is_empty(), "{:#?}", violations);
}

#[tokio::test(start_paused = true)]
async fn sim_fault_injection_detects_real_bug() {
    // Run with fault mode enabled; invariant checks are relaxed.
    // If we then corrupt the backend manually, the final consistency check
    // (which reads raw backends, bypassing faults) should still detect it.
    let fc = FaultConfig {
        error_rate: 0.0,
        corruption_rate: 0.0,
    };
    let mut sim = Sim::new_with_faults(7, Some(fc)).await;
    let _ = sim.run(50).await;

    // Now corrupt backend state directly (not via fault injection)
    sim.agents[0]
        .work_backend
        .write("corrupt.txt", b"oops")
        .await
        .unwrap();

    let violations = check_final_consistency(&sim.agents, &sim.oracle).await;
    assert!(
        !violations.is_empty(),
        "expected violations after real corruption under fault mode"
    );
}

#[tokio::test(start_paused = true)]
async fn sim_fault_injection_corrupts_reads() {
    let fc = FaultConfig {
        error_rate: 0.0,
        corruption_rate: 1.0,
    };
    let mut sim = Sim::new_with_faults(1234, Some(fc)).await;

    let _ = sim
        .step_with(
            0,
            Op::Write {
                mount: MountId::Work,
                path: "corrupt_me.txt".to_string(),
                content: b"clean".to_vec(),
            },
        )
        .await;

    let before = sim.agents[0].fault_stats();
    let _ = sim
        .step_with(
            0,
            Op::Read {
                mount: MountId::Work,
                path: "corrupt_me.txt".to_string(),
            },
        )
        .await;
    let after = sim.agents[0].fault_stats();

    assert!(
        after.corruption_count > before.corruption_count,
        "expected read corruption to be injected"
    );
}

// ─── Write-Back Tests ───────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn sim_write_back_flush_consistency() {
    // Enable write-back for agent 1's indexed mount
    let mut sim = Sim::new_with_config(42, None, true).await;

    // Write some files as agent 1 on indexed mount
    let v = sim
        .step_with(
            1,
            Op::Write {
                mount: MountId::Indexed,
                path: "wb_file_1.txt".to_string(),
                content: b"write-back-data-1".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim
        .step_with(
            1,
            Op::Write {
                mount: MountId::Indexed,
                path: "wb_file_2.txt".to_string(),
                content: b"write-back-data-2".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    // Before flush: reads through cache (router) should work
    let v = sim
        .step_with(
            1,
            Op::Read {
                mount: MountId::Indexed,
                path: "wb_file_1.txt".to_string(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    // Flush write-back
    let v = sim.step_with(1, Op::FlushWriteBack).await;
    assert!(v.is_empty(), "{:#?}", v);

    // After flush: shutdown and check final consistency
    sim.shutdown().await;

    let violations = check_final_consistency(&sim.agents, &sim.oracle).await;
    assert!(violations.is_empty(), "{:#?}", violations);
}

#[tokio::test(start_paused = true)]
async fn sim_write_back_deferred_visibility() {
    // Enable write-back for agent 1
    let mut sim = Sim::new_with_config(100, None, true).await;

    // Write a file as agent 1 on indexed mount
    let v = sim
        .step_with(
            1,
            Op::Write {
                mount: MountId::Indexed,
                path: "deferred.txt".to_string(),
                content: b"pending".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    // The raw backend should NOT have the file yet (write-back hasn't flushed)
    assert!(
        sim.agents[1].indexed_backend.read("deferred.txt").await.is_err(),
        "write-back should not have flushed to backend yet"
    );

    // But reading through the router (cache) should find it
    let (backend, relative, _) = sim.agents[1]
        .router
        .resolve("/a1/indexed/deferred.txt")
        .unwrap();
    let cached = backend.read(&relative).await.unwrap();
    assert_eq!(cached, b"pending");

    // Flush
    let v = sim.step_with(1, Op::FlushWriteBack).await;
    assert!(v.is_empty(), "{:#?}", v);

    // Now the raw backend should have the file
    let raw = sim.agents[1].indexed_backend.read("deferred.txt").await.unwrap();
    assert_eq!(raw, b"pending");
}

// ─── Concurrency Tests ──────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn sim_concurrent_private_mounts() {
    let mut sim = Sim::new(42).await;

    // Scripted: both agents write to their own work mounts (no conflict)
    let v = sim
        .step_with(
            0,
            Op::Write {
                mount: MountId::Work,
                path: "file_a0.txt".to_string(),
                content: b"agent0".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim
        .step_with(
            1,
            Op::Write {
                mount: MountId::Work,
                path: "file_a1.txt".to_string(),
                content: b"agent1".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    // Run concurrent batches — most ops will hit private mounts
    let violations = sim.run_concurrent(50).await;
    assert!(violations.is_empty(), "{:#?}", violations);
}

#[tokio::test(start_paused = true)]
async fn sim_concurrent_shared_write() {
    let mut sim = Sim::new(77).await;

    // Both agents write to the same shared path concurrently
    let v = sim
        .step_with(
            0,
            Op::Write {
                mount: MountId::SharedWrite,
                path: "race.txt".to_string(),
                content: b"from_agent_0".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    // Run concurrent batches
    let violations = sim.run_concurrent(100).await;
    assert!(violations.is_empty(), "{:#?}", violations);
}

#[tokio::test(start_paused = true)]
async fn sim_concurrent_shared_write_same_path_writes() {
    let mut sim = Sim::new(78).await;

    let v = sim
        .step_concurrent_with(
            Op::Write {
                mount: MountId::SharedWrite,
                path: "race.txt".to_string(),
                content: b"from_agent_0".to_vec(),
            },
            Op::Write {
                mount: MountId::SharedWrite,
                path: "race.txt".to_string(),
                content: b"from_agent_1".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let data = sim.agents[0].shared_write.read("race.txt").await.unwrap();
    assert!(data == b"from_agent_0" || data == b"from_agent_1");
}

// ─── Intentional Failure Tests ──────────────────────────────────────────────
// These tests are expected to panic. They deliberately assert the wrong thing
// after corrupting state, so a failure here means the harness stopped detecting
// the corruption.

#[tokio::test(start_paused = true)]
#[should_panic]
async fn sim_intentional_fail_work_backend_corruption() {
    let mut sim = Sim::new(21).await;
    let _ = sim.run(25).await;

    // Corrupt backend state without updating oracle.
    sim.agents[0]
        .work_backend
        .write("intentional_bad.txt", b"oops")
        .await
        .unwrap();

    let violations = check_final_consistency(&sim.agents, &sim.oracle).await;
    // Intentionally wrong: should panic because violations are expected.
    assert!(violations.is_empty(), "{:#?}", violations);
}

#[tokio::test(start_paused = true)]
#[should_panic]
async fn sim_intentional_fail_indexed_backend_corruption() {
    let mut sim = Sim::new(22).await;

    let v = sim
        .step_with(
            0,
            Op::Write {
                mount: MountId::Indexed,
                path: "idx/intentional.txt".to_string(),
                content: b"good".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    // Corrupt indexed backend content directly.
    sim.agents[0]
        .indexed_backend
        .write("idx/intentional.txt", b"bad")
        .await
        .unwrap();

    let violations = check_final_consistency(&sim.agents, &sim.oracle).await;
    // Intentionally wrong: should panic because violations are expected.
    assert!(violations.is_empty(), "{:#?}", violations);
}

#[tokio::test(start_paused = true)]
#[should_panic]
async fn sim_intentional_fail_chroma_corruption() {
    let mut sim = Sim::new(23).await;

    let path = "idx/intentional_chroma.txt".to_string();
    let v = sim
        .step_with(
            0,
            Op::Write {
                mount: MountId::Indexed,
                path: path.clone(),
                content: b"content".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim.step_with(0, Op::IndexFile { path: path.clone() }).await;
    assert!(v.is_empty(), "{:#?}", v);

    // Remove indexed docs from chroma without updating oracle.
    let _ = sim
        .agents[0]
        .chroma
        .delete_by_metadata(json!({"source_path": path}))
        .await
        .unwrap();

    let violations = check_final_consistency(&sim.agents, &sim.oracle).await;
    // Intentionally wrong: should panic because violations are expected.
    assert!(violations.is_empty(), "{:#?}", violations);
}

#[tokio::test(start_paused = true)]
async fn sim_concurrent_shared_write_append_race() {
    let mut sim = Sim::new(79).await;

    let v = sim
        .step_concurrent_with(
            Op::Append {
                mount: MountId::SharedWrite,
                path: "append_race.txt".to_string(),
                content: b"A".to_vec(),
            },
            Op::Append {
                mount: MountId::SharedWrite,
                path: "append_race.txt".to_string(),
                content: b"B".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let data = sim.agents[0]
        .shared_write
        .read("append_race.txt")
        .await
        .unwrap();
    assert!(data == b"AB" || data == b"BA");
}

#[tokio::test(start_paused = true)]
async fn sim_concurrent_shared_write_read_race() {
    let mut sim = Sim::new(80).await;

    let v = sim
        .step_with(
            0,
            Op::Write {
                mount: MountId::SharedWrite,
                path: "read_race.txt".to_string(),
                content: b"old".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim
        .step_concurrent_with(
            Op::Read {
                mount: MountId::SharedWrite,
                path: "read_race.txt".to_string(),
            },
            Op::Write {
                mount: MountId::SharedWrite,
                path: "read_race.txt".to_string(),
                content: b"new".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);
}

#[tokio::test(start_paused = true)]
async fn sim_concurrent_fuzz_50_seeds() {
    for seed in 0..50 {
        let mut sim = Sim::new(seed).await;
        let violations = sim.run_concurrent(30).await;
        assert!(violations.is_empty(), "seed {}: {:#?}", seed, violations);
    }
}

// ─── Combined Tests ─────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn sim_fault_injection_with_write_back() {
    let fc = FaultConfig {
        error_rate: 0.05,
        corruption_rate: 0.0,
    };
    let mut sim = Sim::new_with_config(42, Some(fc), true).await;
    let violations = sim.run(200).await;
    assert!(violations.is_empty(), "{:#?}", violations);
}

#[tokio::test(start_paused = true)]
async fn sim_concurrent_with_faults() {
    let fc = FaultConfig {
        error_rate: 0.10,
        corruption_rate: 0.0,
    };
    let mut sim = Sim::new_with_faults(55, Some(fc)).await;
    let violations = sim.run_concurrent(100).await;
    assert!(violations.is_empty(), "{:#?}", violations);
}
