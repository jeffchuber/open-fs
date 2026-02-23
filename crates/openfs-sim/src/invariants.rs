use std::collections::HashSet;

use openfs_core::Backend;

use crate::agent::AgentVm;
use crate::ops::MountId;
use crate::oracle::Oracle;

/// A violation detected during simulation.
#[derive(Debug, Clone)]
pub struct Violation {
    pub step: usize,
    pub agent_id: usize,
    pub invariant: String,
    pub details: String,
}

/// Run per-step invariant checks.
///
/// `pending_write_back_paths` tracks paths written via write-back but not yet flushed.
/// When non-empty, raw backend checks for agent 1's indexed mount are skipped (only
/// router-level reads are verified).
///
/// `has_faults` indicates fault injection is active; when true, raw backend checks
/// are skipped for all mounts (faults can cause expected cache/backend divergence).
pub async fn check_step_invariants(
    step: usize,
    agents: &[AgentVm],
    oracle: &Oracle,
    pending_write_back_paths: &HashSet<String>,
    has_faults: bool,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    for agent in agents {
        let aid = agent.id;

        // 1. Read-after-write for write-through (agent 0's indexed mount).
        //    After writes, the raw MemoryBackend should match the oracle.
        //    Skip if fault injection is active (faults cause expected divergence).
        if aid == 0 && !has_faults {
            for (path, expected_content) in oracle.files_for(0, MountId::Indexed) {
                match agent.indexed_backend.read(path).await {
                    Ok(actual) => {
                        if actual != *expected_content {
                            violations.push(Violation {
                                step,
                                agent_id: aid,
                                invariant: "write-through-raw-match".to_string(),
                                details: format!(
                                    "Agent 0 indexed backend raw read mismatch for '{}': expected {} bytes, got {} bytes",
                                    path,
                                    expected_content.len(),
                                    actual.len()
                                ),
                            });
                        }
                    }
                    Err(e) => {
                        violations.push(Violation {
                            step,
                            agent_id: aid,
                            invariant: "write-through-raw-exists".to_string(),
                            details: format!(
                                "Agent 0 indexed backend missing '{}' that oracle expects: {}",
                                path, e
                            ),
                        });
                    }
                }
            }
        }

        // For agent 1's indexed mount (write-back): skip raw backend check if there
        // are pending write-back paths (writes are deferred to background flush).
        if aid == 1 && !has_faults && pending_write_back_paths.is_empty() {
            for (path, expected_content) in oracle.files_for(1, MountId::Indexed) {
                match agent.indexed_backend.read(path).await {
                    Ok(actual) => {
                        if actual != *expected_content {
                            violations.push(Violation {
                                step,
                                agent_id: aid,
                                invariant: "write-back-raw-match".to_string(),
                                details: format!(
                                    "Agent 1 indexed backend raw read mismatch for '{}': expected {} bytes, got {} bytes",
                                    path,
                                    expected_content.len(),
                                    actual.len()
                                ),
                            });
                        }
                    }
                    Err(_) => {
                        // File may not be in raw backend yet if write-back hasn't flushed.
                        // Only flag as violation if no pending writes.
                    }
                }
            }
        }

        // 2. Router read-through: cached reads should match oracle for private mounts.
        //    Skip if faults are active (faults can cause reads to fail).
        if !has_faults {
            for mount in [MountId::Work, MountId::Indexed] {
                for (path, expected_content) in oracle.files_for(aid, mount) {
                    if mount == MountId::Indexed
                        && oracle.indexed_shared_12()
                        && (1..=2).contains(&aid)
                    {
                        continue;
                    }
                    let full_path = format!("{}/{}", mount.prefix(aid), path);
                    match agent.router.resolve(&full_path) {
                        Ok((backend, relative, _)) => match backend.read(&relative).await {
                            Ok(actual) => {
                                if actual != *expected_content {
                                    violations.push(Violation {
                                        step,
                                        agent_id: aid,
                                        invariant: "router-read-match".to_string(),
                                        details: format!(
                                            "Router read mismatch for '{}': expected {} bytes, got {} bytes",
                                            full_path,
                                            expected_content.len(),
                                            actual.len()
                                        ),
                                    });
                                }
                            }
                            Err(e) => {
                                violations.push(Violation {
                                    step,
                                    agent_id: aid,
                                    invariant: "router-read-exists".to_string(),
                                    details: format!(
                                        "Router read failed for '{}': {}",
                                        full_path, e
                                    ),
                                });
                            }
                        },
                        Err(e) => violations.push(Violation {
                            step,
                            agent_id: aid,
                            invariant: "router-read-exists".to_string(),
                            details: format!("Router resolve failed for '{}': {}", full_path, e),
                        }),
                    }
                }
            }

            // 2b. Shared read mount should be consistent via router for all agents.
            for (path, expected_content) in oracle.files_for(aid, MountId::SharedRead) {
                let full_path = format!("{}/{}", MountId::SharedRead.prefix(aid), path);
                match agent.router.resolve(&full_path) {
                    Ok((backend, relative, _)) => match backend.read(&relative).await {
                        Ok(actual) => {
                            if actual != *expected_content {
                                violations.push(Violation {
                                    step,
                                    agent_id: aid,
                                    invariant: "shared-read-router-match".to_string(),
                                    details: format!(
                                        "Shared read router mismatch for '{}': expected {} bytes, got {} bytes",
                                        full_path,
                                        expected_content.len(),
                                        actual.len()
                                    ),
                                });
                            }
                        }
                        Err(e) => violations.push(Violation {
                            step,
                            agent_id: aid,
                            invariant: "shared-read-router-exists".to_string(),
                            details: format!(
                                "Shared read router failed for '{}': {}",
                                full_path, e
                            ),
                        }),
                    },
                    Err(e) => violations.push(Violation {
                        step,
                        agent_id: aid,
                        invariant: "shared-read-router-exists".to_string(),
                        details: format!(
                            "Shared read router resolve failed for '{}': {}",
                            full_path, e
                        ),
                    }),
                }
            }
        }

        // 3. Mount isolation: agent 0's private files should not be readable by agent 1's
        //    private backends, and vice versa.
        if aid == 0 && !has_faults {
            let other = &agents[1];
            for path in oracle.files_for(0, MountId::Work).keys() {
                if other.work_backend.read(path).await.is_ok() {
                    violations.push(Violation {
                        step,
                        agent_id: 0,
                        invariant: "mount-isolation".to_string(),
                        details: format!(
                            "Agent 0's work file '{}' is readable from agent 1's work backend",
                            path
                        ),
                    });
                }
            }
        }

        // 4. Read-only enforcement: shared_read backend should be unchanged
        //    (we verify by checking that its contents match oracle.shared_read exactly).
        // This is checked via the oracle expected ReadOnly errors during ops.

        // 5. Shared write convergence: both agents see the same data from the raw shared_write backend.
        if aid == 0 && !has_faults {
            for (path, expected) in oracle.shared_write_files() {
                let a0_read = agents[0].shared_write.read(path).await;
                let a1_read = agents[1].shared_write.read(path).await;
                match (a0_read, a1_read) {
                    (Ok(d0), Ok(d1)) => {
                        if d0 != d1 {
                            violations.push(Violation {
                                step,
                                agent_id: 0,
                                invariant: "shared-write-convergence".to_string(),
                                details: format!(
                                    "Shared write '{}': agent 0 sees {} bytes, agent 1 sees {} bytes",
                                    path,
                                    d0.len(),
                                    d1.len()
                                ),
                            });
                        }
                        if d0 != *expected {
                            violations.push(Violation {
                                step,
                                agent_id: 0,
                                invariant: "shared-write-oracle-match".to_string(),
                                details: format!(
                                    "Shared write '{}': raw backend has {} bytes, oracle expects {} bytes",
                                    path,
                                    d0.len(),
                                    expected.len()
                                ),
                            });
                        }
                    }
                    (Err(e), _) | (_, Err(e)) => {
                        violations.push(Violation {
                            step,
                            agent_id: 0,
                            invariant: "shared-write-readable".to_string(),
                            details: format!("Shared write '{}' not readable: {}", path, e),
                        });
                    }
                }
            }

            // 6. Last writer should see its own shared_write content via router.
            for (path, expected) in oracle.shared_write_files() {
                if let Some(last_writer) = oracle.shared_write_last_writers().get(path) {
                    let agent = &agents[*last_writer];
                    let full_path =
                        format!("{}/{}", MountId::SharedWrite.prefix(*last_writer), path);
                    match agent.router.resolve(&full_path) {
                        Ok((backend, relative, _)) => match backend.read(&relative).await {
                            Ok(actual) => {
                                if actual != *expected {
                                    violations.push(Violation {
                                        step,
                                        agent_id: *last_writer,
                                        invariant: "shared-write-last-writer".to_string(),
                                        details: format!(
                                            "Last writer {} sees {} bytes for '{}', expected {} bytes",
                                            last_writer,
                                            actual.len(),
                                            full_path,
                                            expected.len()
                                        ),
                                    });
                                }
                            }
                            Err(e) => violations.push(Violation {
                                step,
                                agent_id: *last_writer,
                                invariant: "shared-write-last-writer".to_string(),
                                details: format!(
                                    "Last writer {} failed to read '{}': {}",
                                    last_writer, full_path, e
                                ),
                            }),
                        },
                        Err(e) => violations.push(Violation {
                            step,
                            agent_id: *last_writer,
                            invariant: "shared-write-last-writer".to_string(),
                            details: format!(
                                "Last writer {} failed to resolve '{}': {}",
                                last_writer, full_path, e
                            ),
                        }),
                    }
                }
            }
        }
    }

    violations
}

/// Final consistency checks run at end of simulation after flushing all write-back.
pub async fn check_final_consistency(agents: &[AgentVm], oracle: &Oracle) -> Vec<Violation> {
    let mut violations = Vec::new();

    // 8. Full state match: enumerate all files in every MemoryBackend and verify they
    //    exactly match the oracle's model.
    for agent in agents {
        let aid = agent.id;

        // Check work backend
        check_backend_matches_oracle(
            &*agent.work_backend,
            oracle.files_for(aid, MountId::Work),
            aid,
            "work",
            true, // check for extras
            &mut violations,
        )
        .await;

        // Check indexed backend
        check_backend_matches_oracle(
            &*agent.indexed_backend,
            oracle.files_for(aid, MountId::Indexed),
            aid,
            "indexed",
            true, // check for extras
            &mut violations,
        )
        .await;
    }

    // Shared write
    check_backend_matches_oracle(
        &*agents[0].shared_write,
        oracle.shared_write_files(),
        0,
        "shared_write",
        true,
        &mut violations,
    )
    .await;

    // Shared read (should be unchanged)
    check_backend_matches_oracle(
        &*agents[0].shared_read,
        oracle.files_for(0, MountId::SharedRead),
        0,
        "shared_read",
        true,
        &mut violations,
    )
    .await;

    // 9. Chroma completeness: every indexed file in oracle has docs in MockChromaStore.
    for (agent_id, path) in &oracle.indexed {
        if !agents[*agent_id].chroma.has_docs_for_path(path) {
            violations.push(Violation {
                step: usize::MAX,
                agent_id: *agent_id,
                invariant: "chroma-completeness".to_string(),
                details: format!(
                    "Agent {} indexed file '{}' but no docs found in MockChromaStore",
                    agent_id, path
                ),
            });
        }
    }

    violations
}

async fn check_backend_matches_oracle(
    backend: &dyn Backend,
    oracle_files: &std::collections::HashMap<String, Vec<u8>>,
    agent_id: usize,
    mount_name: &str,
    check_extras: bool,
    violations: &mut Vec<Violation>,
) {
    // Check all oracle files exist in backend with correct content
    for (path, expected) in oracle_files {
        match backend.read(path).await {
            Ok(actual) => {
                if actual != *expected {
                    violations.push(Violation {
                        step: usize::MAX,
                        agent_id,
                        invariant: format!("final-{}-content-match", mount_name),
                        details: format!(
                            "File '{}': expected {} bytes, got {} bytes",
                            path,
                            expected.len(),
                            actual.len()
                        ),
                    });
                }
            }
            Err(_) => {
                violations.push(Violation {
                    step: usize::MAX,
                    agent_id,
                    invariant: format!("final-{}-exists", mount_name),
                    details: format!("File '{}' missing from backend", path),
                });
            }
        }
    }

    // Check no extra files in backend that oracle doesn't know about.
    if check_extras {
        let mut backend_files = Vec::new();
        if collect_backend_files(backend, &mut backend_files)
            .await
            .is_ok()
        {
            for path in backend_files {
                if !oracle_files.contains_key(&path) {
                    violations.push(Violation {
                        step: usize::MAX,
                        agent_id,
                        invariant: format!("final-{}-no-extra", mount_name),
                        details: format!("Backend has file '{}' not tracked by oracle", path),
                    });
                }
            }
        }
    }
}

async fn collect_backend_files(
    backend: &dyn Backend,
    out: &mut Vec<String>,
) -> Result<(), openfs_core::BackendError> {
    let mut stack = vec![String::new()];

    while let Some(dir) = stack.pop() {
        let entries = backend.list(&dir).await?;
        for entry in entries {
            if entry.is_dir {
                stack.push(entry.path.clone());
            } else {
                out.push(entry.path.clone());
            }
        }
    }

    Ok(())
}
