use std::env;
use std::error::Error;
use std::path::PathBuf;

use openfs_sim::coding_agents_sim::{self, CodingAgentSim, CodingSimProfile};

#[derive(Debug, Clone)]
struct Config {
    seed: u64,
    agents: usize,
    rounds: usize,
    out_dir: PathBuf,
    profile: CodingSimProfile,
}

fn print_usage(program: &str) {
    eprintln!(
        "Usage: {program} [--seed N] [--agents N] [--rounds N] [--profile balanced|cas] [--out DIR]\n\
         Example: {program} --seed 42 --agents 8 --rounds 30 --profile cas --out /tmp/openfs-coding-sim"
    );
}

fn parse_args() -> Result<Config, String> {
    let mut args = env::args().skip(1);
    let mut config = Config {
        seed: 42,
        agents: 8,
        rounds: 30,
        out_dir: PathBuf::from("/tmp/openfs-coding-sim"),
        profile: CodingSimProfile::CasDemo,
    };

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seed" => {
                let value = args.next().ok_or("--seed requires a value")?;
                config.seed = value
                    .parse::<u64>()
                    .map_err(|e| format!("invalid --seed '{}': {}", value, e))?;
            }
            "--agents" => {
                let value = args.next().ok_or("--agents requires a value")?;
                config.agents = value
                    .parse::<usize>()
                    .map_err(|e| format!("invalid --agents '{}': {}", value, e))?;
            }
            "--rounds" => {
                let value = args.next().ok_or("--rounds requires a value")?;
                config.rounds = value
                    .parse::<usize>()
                    .map_err(|e| format!("invalid --rounds '{}': {}", value, e))?;
            }
            "--out" => {
                let value = args.next().ok_or("--out requires a value")?;
                config.out_dir = PathBuf::from(value);
            }
            "--profile" => {
                let value = args.next().ok_or("--profile requires a value")?;
                config.profile = match value.as_str() {
                    "balanced" => CodingSimProfile::Balanced,
                    "cas" | "cas-demo" => CodingSimProfile::CasDemo,
                    _ => {
                        return Err(format!(
                            "invalid --profile '{}': expected balanced or cas",
                            value
                        ));
                    }
                };
            }
            "-h" | "--help" => {
                let program = env::args()
                    .next()
                    .unwrap_or_else(|| "coding_agents_sim".to_string());
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
            let program = env::args()
                .next()
                .unwrap_or_else(|| "coding_agents_sim".to_string());
            print_usage(&program);
            return Err(err.into());
        }
    };

    let mut sim =
        CodingAgentSim::new_with_profile(config.seed, config.agents, config.profile).await;
    sim.run_rounds(config.rounds).await;
    let summary = sim.summary().await?;
    let report_path = coding_agents_sim::write_report(&sim, &config.out_dir).await?;
    let (html_path, json_path) = coding_agents_sim::write_bundle(&sim, &config.out_dir).await?;

    println!("OpenFS coding-agent sim report generated.");
    println!(
        "seed={} agents={} rounds={} profile={}",
        config.seed,
        summary.agent_count,
        summary.rounds,
        summary.profile.as_str()
    );
    println!(
        "trace_entries={} errors={} cas_conflicts={} cas_retried_writes={} cas_max_retry={}",
        summary.trace_entries,
        summary.error_count,
        summary.cas_conflicts,
        summary.cas_retried_writes,
        summary.cas_max_retries
    );
    println!(
        "code_files={} skill_files={} memory_agents={}/{}",
        summary.code_file_count,
        summary.shared_skill_file_count,
        summary.agents_with_memory,
        summary.agent_count
    );
    println!(
        "contributors: code={} skills={} memory_entries={}",
        summary.code_contributors, summary.skill_contributors, summary.total_memory_entries
    );
    println!("report={}", report_path.display());
    println!("html={}", html_path.display());
    println!("json={}", json_path.display());

    Ok(())
}
