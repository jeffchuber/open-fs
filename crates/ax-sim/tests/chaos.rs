use ax_sim::fault::FaultStats;
use ax_sim::invariants::{check_final_consistency, Violation};
use ax_sim::ops::{MountId, Op};
use ax_sim::{FaultConfig, Sim};
use proptest::prelude::*;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

fn aggregate_faults(sim: &Sim) -> FaultStats {
    let s0 = sim.agents[0].fault_stats();
    let s1 = sim.agents[1].fault_stats();
    FaultStats {
        fault_count: s0.fault_count + s1.fault_count,
        corruption_count: s0.corruption_count + s1.corruption_count,
    }
}

fn run_mixed_case(
    seed: u64,
    steps: usize,
    concurrent_ratio: f64,
    fault_config: Option<FaultConfig>,
    write_back: bool,
) -> (Vec<Violation>, FaultStats) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async move {
        tokio::time::pause();
        let mut sim = Sim::new_with_config(seed, fault_config, write_back).await;
        let violations = sim.run_mixed(steps, concurrent_ratio).await.to_vec();
        let stats = aggregate_faults(&sim);
        (violations, stats)
    })
}

fn run_forced_flush_case(seed: u64, steps: usize, concurrent_ratio: f64) -> Vec<Violation> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async move {
        tokio::time::pause();
        let mut sim = Sim::new_with_config(seed ^ 0xA11C_E5EE_D00D_BAAD, None, true).await;
        let mut rng = ChaCha8Rng::seed_from_u64(seed.wrapping_add(0x9E37_79B9_7F4A_7C15));
        let ratio = if concurrent_ratio.is_finite() {
            concurrent_ratio.clamp(0.0, 1.0)
        } else {
            0.0
        };

        for step in 0..steps {
            // Force regular flushes so write-back drain paths are exercised heavily.
            if step % 5 == 0 {
                let _ = sim.step_with(1, Op::FlushWriteBack).await;
                continue;
            }

            if rng.gen_bool(ratio) {
                let op0 = random_op(&mut rng);
                let op1 = if rng.gen_bool(0.35) {
                    Op::FlushWriteBack
                } else {
                    random_op(&mut rng)
                };
                let _ = sim.step_concurrent_with(op0, op1).await;
            } else {
                let agent_id = if rng.gen_bool(0.5) { 0 } else { 1 };
                let op = if agent_id == 1 && rng.gen_bool(0.25) {
                    Op::FlushWriteBack
                } else {
                    random_op(&mut rng)
                };
                let _ = sim.step_with(agent_id, op).await;
            }
        }

        sim.shutdown().await;
        let mut violations = sim.violations.clone();
        violations.extend(check_final_consistency(&sim.agents, &sim.oracle).await);
        violations
    })
}

fn random_path(rng: &mut ChaCha8Rng, prefix: &str) -> String {
    let depth = rng.gen_range(0..=2);
    let mut parts = Vec::new();
    for i in 0..depth {
        parts.push(format!("dir{}_{}", prefix, i));
    }
    parts.push(format!("{}_file_{}.txt", prefix, rng.gen::<u16>()));
    parts.join("/")
}

fn random_content(rng: &mut ChaCha8Rng) -> Vec<u8> {
    let len = rng.gen_range(1..=32);
    let mut out = vec![0u8; len];
    rng.fill(out.as_mut_slice());
    out
}

fn random_mount(rng: &mut ChaCha8Rng) -> MountId {
    match rng.gen_range(0..4) {
        0 => MountId::Work,
        1 => MountId::Indexed,
        2 => MountId::SharedRead,
        _ => MountId::SharedWrite,
    }
}

fn random_op(rng: &mut ChaCha8Rng) -> Op {
    match rng.gen_range(0..12) {
        0 => Op::Write {
            mount: random_mount(rng),
            path: random_path(rng, "w"),
            content: random_content(rng),
        },
        1 => Op::Read {
            mount: random_mount(rng),
            path: random_path(rng, "r"),
        },
        2 => Op::Append {
            mount: random_mount(rng),
            path: random_path(rng, "a"),
            content: random_content(rng),
        },
        3 => Op::Delete {
            mount: random_mount(rng),
            path: random_path(rng, "d"),
        },
        4 => Op::List {
            mount: random_mount(rng),
            path: if rng.gen_bool(0.5) {
                String::new()
            } else {
                format!("dir{}", rng.gen::<u8>() % 3)
            },
        },
        5 => Op::Stat {
            mount: random_mount(rng),
            path: random_path(rng, "s"),
        },
        6 => Op::Exists {
            mount: random_mount(rng),
            path: random_path(rng, "e"),
        },
        7 => Op::Rename {
            mount: random_mount(rng),
            from: random_path(rng, "from"),
            to: random_path(rng, "to"),
        },
        8 => Op::IndexFile {
            path: random_path(rng, "idx"),
        },
        9 => Op::SearchChroma {
            query: format!("q{}", rng.gen::<u32>()),
        },
        10 => Op::FlushWriteBack,
        _ => Op::Read {
            mount: MountId::SharedRead,
            path: format!("seed_{}.txt", rng.gen::<u8>() % 5),
        },
    }
}

#[tokio::test(start_paused = true)]
async fn chaos_all_ops_scripted() {
    let mut sim = Sim::new_with_config(4242, None, true).await;

    let ops: Vec<(usize, Op)> = vec![
        (
            0,
            Op::Write {
                mount: MountId::Work,
                path: "w0.txt".to_string(),
                content: b"w0".to_vec(),
            },
        ),
        (
            0,
            Op::Read {
                mount: MountId::Work,
                path: "w0.txt".to_string(),
            },
        ),
        (
            0,
            Op::Append {
                mount: MountId::Work,
                path: "w0.txt".to_string(),
                content: b"+".to_vec(),
            },
        ),
        (
            0,
            Op::Exists {
                mount: MountId::Work,
                path: "w0.txt".to_string(),
            },
        ),
        (
            0,
            Op::Stat {
                mount: MountId::Work,
                path: "w0.txt".to_string(),
            },
        ),
        (
            0,
            Op::List {
                mount: MountId::Work,
                path: "".to_string(),
            },
        ),
        (
            0,
            Op::Rename {
                mount: MountId::Work,
                from: "w0.txt".to_string(),
                to: "w0_renamed.txt".to_string(),
            },
        ),
        (
            0,
            Op::Delete {
                mount: MountId::Work,
                path: "w0_renamed.txt".to_string(),
            },
        ),
        (
            0,
            Op::Write {
                mount: MountId::Indexed,
                path: "i0.txt".to_string(),
                content: b"i0".to_vec(),
            },
        ),
        (
            0,
            Op::Append {
                mount: MountId::Indexed,
                path: "i0.txt".to_string(),
                content: b"+".to_vec(),
            },
        ),
        (
            0,
            Op::IndexFile {
                path: "i0.txt".to_string(),
            },
        ),
        (
            0,
            Op::Read {
                mount: MountId::Indexed,
                path: "i0.txt".to_string(),
            },
        ),
        (
            0,
            Op::Rename {
                mount: MountId::Indexed,
                from: "i0.txt".to_string(),
                to: "i0_renamed.txt".to_string(),
            },
        ),
        (
            0,
            Op::Delete {
                mount: MountId::Indexed,
                path: "i0_renamed.txt".to_string(),
            },
        ),
        (
            1,
            Op::Write {
                mount: MountId::Indexed,
                path: "i1.txt".to_string(),
                content: b"i1".to_vec(),
            },
        ),
        (
            1,
            Op::Append {
                mount: MountId::Indexed,
                path: "i1.txt".to_string(),
                content: b"+".to_vec(),
            },
        ),
        (1, Op::FlushWriteBack),
        (
            1,
            Op::Read {
                mount: MountId::Indexed,
                path: "i1.txt".to_string(),
            },
        ),
        (
            1,
            Op::IndexFile {
                path: "i1.txt".to_string(),
            },
        ),
        (
            1,
            Op::Rename {
                mount: MountId::Indexed,
                from: "i1.txt".to_string(),
                to: "i1_renamed.txt".to_string(),
            },
        ),
        (
            1,
            Op::Delete {
                mount: MountId::Indexed,
                path: "i1_renamed.txt".to_string(),
            },
        ),
        (
            0,
            Op::Read {
                mount: MountId::SharedRead,
                path: "seed_0.txt".to_string(),
            },
        ),
        (
            0,
            Op::Exists {
                mount: MountId::SharedRead,
                path: "seed_0.txt".to_string(),
            },
        ),
        (
            0,
            Op::Stat {
                mount: MountId::SharedRead,
                path: "seed_0.txt".to_string(),
            },
        ),
        (
            0,
            Op::List {
                mount: MountId::SharedRead,
                path: "".to_string(),
            },
        ),
        (
            0,
            Op::Write {
                mount: MountId::SharedRead,
                path: "ro.txt".to_string(),
                content: b"no".to_vec(),
            },
        ),
        (
            0,
            Op::Append {
                mount: MountId::SharedRead,
                path: "seed_0.txt".to_string(),
                content: b"no".to_vec(),
            },
        ),
        (
            0,
            Op::Delete {
                mount: MountId::SharedRead,
                path: "seed_0.txt".to_string(),
            },
        ),
        (
            0,
            Op::Rename {
                mount: MountId::SharedRead,
                from: "seed_0.txt".to_string(),
                to: "seed_0_new.txt".to_string(),
            },
        ),
        (
            0,
            Op::Write {
                mount: MountId::SharedWrite,
                path: "sw.txt".to_string(),
                content: b"sw".to_vec(),
            },
        ),
        (
            1,
            Op::Read {
                mount: MountId::SharedWrite,
                path: "sw.txt".to_string(),
            },
        ),
        (
            1,
            Op::Append {
                mount: MountId::SharedWrite,
                path: "sw.txt".to_string(),
                content: b"+".to_vec(),
            },
        ),
        (
            0,
            Op::Exists {
                mount: MountId::SharedWrite,
                path: "sw.txt".to_string(),
            },
        ),
        (
            0,
            Op::Stat {
                mount: MountId::SharedWrite,
                path: "sw.txt".to_string(),
            },
        ),
        (
            1,
            Op::List {
                mount: MountId::SharedWrite,
                path: "".to_string(),
            },
        ),
        (
            0,
            Op::Rename {
                mount: MountId::SharedWrite,
                from: "sw.txt".to_string(),
                to: "sw2.txt".to_string(),
            },
        ),
        (
            1,
            Op::Delete {
                mount: MountId::SharedWrite,
                path: "sw2.txt".to_string(),
            },
        ),
        (
            0,
            Op::SearchChroma {
                query: "smoke_q".to_string(),
            },
        ),
        (
            1,
            Op::SearchChroma {
                query: "smoke_q2".to_string(),
            },
        ),
    ];

    for (agent_id, op) in ops {
        let v = sim.step_with(agent_id, op).await;
        assert!(v.is_empty(), "{:#?}", v);
    }
}

#[tokio::test(start_paused = true)]
async fn chaos_all_ops_concurrent() {
    let mut sim = Sim::new_with_config(7777, None, true).await;

    let v = sim
        .step_with(
            0,
            Op::Write {
                mount: MountId::SharedWrite,
                path: "c_sw.txt".to_string(),
                content: b"c0".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let v = sim
        .step_with(
            1,
            Op::Write {
                mount: MountId::Indexed,
                path: "c_i1.txt".to_string(),
                content: b"c1".to_vec(),
            },
        )
        .await;
    assert!(v.is_empty(), "{:#?}", v);

    let pairs = vec![
        (
            Op::Write {
                mount: MountId::SharedWrite,
                path: "c_sw.txt".to_string(),
                content: b"c0a".to_vec(),
            },
            Op::Write {
                mount: MountId::SharedWrite,
                path: "c_sw.txt".to_string(),
                content: b"c0b".to_vec(),
            },
        ),
        (
            Op::Read {
                mount: MountId::SharedWrite,
                path: "c_sw.txt".to_string(),
            },
            Op::Append {
                mount: MountId::SharedWrite,
                path: "c_sw.txt".to_string(),
                content: b"+".to_vec(),
            },
        ),
        (
            Op::List {
                mount: MountId::SharedWrite,
                path: "".to_string(),
            },
            Op::Read {
                mount: MountId::SharedRead,
                path: "seed_1.txt".to_string(),
            },
        ),
        (
            Op::Write {
                mount: MountId::Indexed,
                path: "c_i0.txt".to_string(),
                content: b"c0".to_vec(),
            },
            Op::FlushWriteBack,
        ),
        (
            Op::Rename {
                mount: MountId::Indexed,
                from: "c_i1.txt".to_string(),
                to: "c_i1_renamed.txt".to_string(),
            },
            Op::Read {
                mount: MountId::Indexed,
                path: "c_i1_renamed.txt".to_string(),
            },
        ),
        (
            Op::SearchChroma {
                query: "cq".to_string(),
            },
            Op::IndexFile {
                path: "c_i0.txt".to_string(),
            },
        ),
    ];

    for (op0, op1) in pairs {
        let v = sim.step_concurrent_with(op0, op1).await;
        assert!(v.is_empty(), "{:#?}", v);
    }
}

#[tokio::test(start_paused = true)]
async fn chaos_op_soup_seeded() {
    let mut sim = Sim::new_with_config(9999, None, true).await;
    let mut rng = ChaCha8Rng::seed_from_u64(0xC0FFEE);

    let mut ops: Vec<Op> = Vec::new();
    for _ in 0..300 {
        ops.push(random_op(&mut rng));
    }

    let mut idx = 0usize;
    while idx < ops.len() {
        if rng.gen_bool(0.25) && idx + 1 < ops.len() {
            let v = sim
                .step_concurrent_with(ops[idx].clone(), ops[idx + 1].clone())
                .await;
            assert!(v.is_empty(), "{:#?}", v);
            idx += 2;
        } else {
            let agent_id = if rng.gen_bool(0.5) { 0 } else { 1 };
            let v = sim.step_with(agent_id, ops[idx].clone()).await;
            assert!(v.is_empty(), "{:#?}", v);
            idx += 1;
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        max_shrink_iters: 256,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_sim_mixed_no_faults(
        seed in any::<u64>(),
        steps in 1usize..120,
        write_back in any::<bool>(),
        concurrent_pct in 0u8..=80u8,
    ) {
        let ratio = (concurrent_pct as f64) / 100.0;
        let (violations, stats) = run_mixed_case(seed, steps, ratio, None, write_back);
        prop_assert!(
            violations.is_empty(),
            "seed {} steps {} write_back {} concurrent {}%: {:#?}",
            seed,
            steps,
            write_back,
            concurrent_pct,
            violations
        );
        prop_assert_eq!(stats.fault_count, 0);
        prop_assert_eq!(stats.corruption_count, 0);
    }

    #[test]
    fn prop_sim_mixed_with_faults(
        seed in any::<u64>(),
        steps in 1usize..80,
        write_back in any::<bool>(),
        concurrent_pct in 0u8..=80u8,
    ) {
        let ratio = (concurrent_pct as f64) / 100.0;
        let fc = FaultConfig {
            error_rate: 0.35,
            corruption_rate: 0.15,
        };
        let (violations, _stats) = run_mixed_case(seed, steps, ratio, Some(fc), write_back);
        prop_assert!(
            violations.is_empty(),
            "seed {} steps {} write_back {} concurrent {}%: {:#?}",
            seed,
            steps,
            write_back,
            concurrent_pct,
            violations
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        max_shrink_iters: 128,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_sim_write_back_forced_flush(
        seed in any::<u64>(),
        steps in 20usize..180,
        concurrent_pct in 0u8..=90u8,
    ) {
        let ratio = (concurrent_pct as f64) / 100.0;
        let violations = run_forced_flush_case(seed, steps, ratio);
        prop_assert!(
            violations.is_empty(),
            "seed {} steps {} forced-flush concurrent {}%: {:#?}",
            seed,
            steps,
            concurrent_pct,
            violations
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 12,
        max_shrink_iters: 128,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_sim_mixed_no_faults_longer(
        seed in any::<u64>(),
        steps in 120usize..300,
        concurrent_pct in 0u8..=100u8,
    ) {
        let ratio = (concurrent_pct as f64) / 100.0;
        let (violations, _stats) = run_mixed_case(seed, steps, ratio, None, true);
        prop_assert!(
            violations.is_empty(),
            "seed {} steps {} write_back true concurrent {}%: {:#?}",
            seed,
            steps,
            concurrent_pct,
            violations
        );
    }

    #[test]
    fn prop_sim_mixed_with_faults_aggressive(
        seed in any::<u64>(),
        steps in 40usize..140,
        concurrent_pct in 0u8..=100u8,
    ) {
        let ratio = (concurrent_pct as f64) / 100.0;
        let fc = FaultConfig {
            error_rate: 0.60,
            corruption_rate: 0.30,
        };
        let (violations, _stats) = run_mixed_case(seed, steps, ratio, Some(fc), true);
        prop_assert!(
            violations.is_empty(),
            "seed {} steps {} write_back true concurrent {}%: {:#?}",
            seed,
            steps,
            concurrent_pct,
            violations
        );
    }
}

#[test]
fn chaos_seed_regressions() {
    let fc = FaultConfig {
        error_rate: 0.35,
        corruption_rate: 0.15,
    };
    let cases = [
        (6935771541855252821u64, 62usize, 0.08f64, true),
        (6935126213708383462u64, 61usize, 0.05f64, true),
        (7048677074468715094u64, 83usize, 0.06f64, true),
        (2052725270671105519u64, 19usize, 0.39f64, true),
        (10774011904878452589u64, 21usize, 0.13f64, true),
    ];

    for (seed, steps, ratio, write_back) in cases {
        let (violations, _stats) = run_mixed_case(seed, steps, ratio, Some(fc.clone()), write_back);
        assert!(
            violations.is_empty(),
            "seed {} steps {} write_back {} concurrent {}%: {:#?}",
            seed,
            steps,
            write_back,
            (ratio * 100.0).round() as u64,
            violations
        );
    }
}
