use std::env;
use std::error::Error;
use std::path::PathBuf;

use openfs_sim::debug_ui;
use openfs_sim::ops::{MountId, Op};
use openfs_sim::Sim;

#[derive(Debug, Clone, Copy)]
enum Mode {
    Sequential,
    Mixed,
    Concurrent,
}

#[derive(Debug, Clone)]
struct Config {
    steps: usize,
    seed: u64,
    mode: Mode,
    concurrent_ratio: f64,
    write_back: bool,
    out_dir: PathBuf,
}

fn print_usage(program: &str) {
    eprintln!(
        "Usage: {program} [--steps N] [--seed N] [--mode sequential|mixed|concurrent] [--concurrent-ratio F] [--write-back|--no-write-back] [--out DIR]"
    );
}

fn parse_args() -> Result<Config, String> {
    let mut args = env::args().skip(1);

    let mut config = Config {
        steps: 150,
        seed: 42,
        mode: Mode::Mixed,
        concurrent_ratio: 0.35,
        write_back: true,
        out_dir: PathBuf::from("/tmp/openfs-sim-debug"),
    };

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--steps" => {
                let value = args.next().ok_or("--steps requires a value")?;
                config.steps = value
                    .parse::<usize>()
                    .map_err(|e| format!("invalid --steps '{}': {}", value, e))?;
            }
            "--seed" => {
                let value = args.next().ok_or("--seed requires a value")?;
                config.seed = value
                    .parse::<u64>()
                    .map_err(|e| format!("invalid --seed '{}': {}", value, e))?;
            }
            "--mode" => {
                let value = args.next().ok_or("--mode requires a value")?;
                config.mode = match value.as_str() {
                    "sequential" => Mode::Sequential,
                    "mixed" => Mode::Mixed,
                    "concurrent" => Mode::Concurrent,
                    _ => {
                        return Err(format!(
                            "invalid --mode '{}', expected sequential|mixed|concurrent",
                            value
                        ))
                    }
                };
            }
            "--concurrent-ratio" => {
                let value = args.next().ok_or("--concurrent-ratio requires a value")?;
                config.concurrent_ratio = value
                    .parse::<f64>()
                    .map_err(|e| format!("invalid --concurrent-ratio '{}': {}", value, e))?;
            }
            "--write-back" => {
                config.write_back = true;
            }
            "--no-write-back" => {
                config.write_back = false;
            }
            "--out" => {
                let value = args.next().ok_or("--out requires a value")?;
                config.out_dir = PathBuf::from(value);
            }
            "-h" | "--help" => {
                let program = env::args().next().unwrap_or_else(|| "debug_ui".to_string());
                print_usage(&program);
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument '{}'", arg)),
        }
    }

    Ok(config)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = match parse_args() {
        Ok(cfg) => cfg,
        Err(err) => {
            let program = env::args().next().unwrap_or_else(|| "debug_ui".to_string());
            print_usage(&program);
            return Err(err.into());
        }
    };

    tokio::time::pause();

    let mut sim = Sim::new_with_remote_client(config.seed, None, config.write_back).await;

    // Seed dedicated remote-writer client (agent 2) with explicit remote0 writes.
    if sim.agents.len() > 2 {
        for i in 0..4usize {
            let path = format!("remote0_seed_{}.txt", i);
            let content = format!("remote0 seeded {}", i).into_bytes();
            let _ = sim
                .step_with(
                    2,
                    Op::Write {
                        mount: MountId::Indexed,
                        path,
                        content,
                    },
                )
                .await;
        }
    }

    let violation_count = match config.mode {
        Mode::Sequential => sim.run(config.steps).await.len(),
        Mode::Mixed => sim
            .run_mixed(config.steps, config.concurrent_ratio)
            .await
            .len(),
        Mode::Concurrent => sim.run_concurrent(config.steps).await.len(),
    };

    let trace_count = sim.trace.len();
    let (html_path, json_path) = debug_ui::write_bundle(&sim, &config.out_dir).await?;

    println!("OpenFS sim debug bundle generated.");
    println!(
        "steps={} seed={} write_back={}",
        config.steps, config.seed, config.write_back
    );
    println!("agents={}", sim.agents.len());
    println!(
        "trace_entries={} violations={}",
        trace_count, violation_count
    );
    println!("html={}", html_path.display());
    println!("json={}", json_path.display());

    Ok(())
}
