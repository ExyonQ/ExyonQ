/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
mod perf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use perf::contract::{contract_command, ContractArgs};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser)]
#[command(name = "xtask")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run fmt, clippy, and tests (CI check).
    Ci,
    /// Security audit gates (deps, tests, release).
    Security {
        #[command(subcommand)]
        command: SecurityCommands,
    },
    /// Comparative benchmark vs Tier A rivals.
    Bench {
        #[command(subcommand)]
        command: BenchCommands,
    },
    /// Dependency upstream checks.
    Deps {
        #[command(subcommand)]
        command: DepsCommands,
    },
    /// Plan 03 performance contract gate (ExyonQ-only baseline compare).
    Perf {
        #[command(subcommand)]
        command: PerfCommands,
    },
}

#[derive(Subcommand)]
enum PerfCommands {
    /// Baseline compare gate (`--quick` default profile).
    Contract(ContractArgs),
}

#[derive(Subcommand)]
enum SecurityCommands {
    /// Run cargo audit, cargo deny, and security integration tests.
    Check,
    /// Generate audit skeleton from template for a release version.
    Audit {
        #[arg(long)]
        version: String,
    },
    /// Pre-release gate: check + verify audit-vX.Y.Z.md exists.
    PreRelease {
        #[arg(long)]
        version: String,
    },
}

#[derive(Subcommand)]
enum BenchCommands {
    /// Full compare: docker compose + functional + perf.
    Compare {
        #[arg(long)]
        functional_only: bool,
        #[arg(long)]
        skip_up: bool,
        #[arg(long, default_value = "30s")]
        duration: String,
        #[arg(
            long,
            default_value = "ceiling",
            help = "ceiling (Modo A) or fixed-rps (Modo B)"
        )]
        mode: String,
        #[arg(long, default_value_t = 60, help = "Tier % for fixed-rps (30, 60, 90)")]
        tier: u8,
        #[arg(long, help = "Force perf from host (macOS from-host)")]
        from_host: bool,
        #[arg(long, help = "Force perf via bench-runner on Docker network")]
        in_docker: bool,
        #[arg(long, help = "Include roadmap scenarios P8-P12")]
        include_roadmap: bool,
        #[arg(long, help = "Benchmark all Tier A servers (default: ExyonQ only)")]
        all_servers: bool,
        #[arg(
            long,
            help = "Measure all rivals live; disables rivals cache (implies --all-servers)"
        )]
        no_cache: bool,
        #[arg(long, help = "Re-measure rivals and refresh the rivals cache")]
        refresh_rivals: bool,
    },
    /// Compute fixed-RPS targets from rivals-cache Modo A ceilings.
    CalibrateTiers {
        #[arg(long, default_value = "30s")]
        duration: String,
    },
    /// Micro-scan P4/P6 at 80/90/100% of ExyonQ Modo A peak (internal).
    PeakScan {
        #[arg(long, default_value = "5m")]
        duration: String,
        #[arg(long, default_value = "20s")]
        warmup: String,
        #[arg(long, help = "Optional Modo A run dir for peak RPS baseline")]
        peak_source: Option<PathBuf>,
        #[arg(long, help = "Capture perf profiles on Linux after each point")]
        profile: bool,
        #[arg(long, help = "Override measure window seconds (measure override)")]
        point_duration_sec: Option<u64>,
    },
    /// Functional F1-F8 only.
    Functional,
    /// Performance scenarios only.
    Perf {
        #[arg(long, default_value = "30s")]
        duration: String,
        #[arg(long, default_value = "ceiling")]
        mode: String,
        #[arg(long, default_value_t = 60)]
        tier: u8,
        #[arg(long)]
        from_host: bool,
        #[arg(long)]
        in_docker: bool,
        #[arg(long)]
        include_roadmap: bool,
        #[arg(long, help = "Benchmark all Tier A servers (default: ExyonQ only)")]
        all_servers: bool,
        #[arg(
            long,
            help = "Measure all rivals live; disables rivals cache (implies --all-servers)"
        )]
        no_cache: bool,
        #[arg(long, help = "Re-measure rivals and refresh the rivals cache")]
        refresh_rivals: bool,
    },
    /// Measure CPU/RAM for Tier A rivals (sequential); updates rivals cache.
    RivalResources {
        #[arg(long, default_value = "30s")]
        duration: String,
    },
}

#[derive(Subcommand)]
enum DepsCommands {
    /// Report pinned vs latest Wasmtime and advisories (signal mode).
    WasmtimeCheck,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Ci => ci()?,
        Commands::Security { command } => security(command)?,
        Commands::Bench { command } => bench(command)?,
        Commands::Deps { command } => deps(command)?,
        Commands::Perf { command } => {
            let root = repo_root()?;
            let code = match command {
                PerfCommands::Contract(args) => contract_command(&root, args)?,
            };
            if code != 0 {
                std::process::exit(code);
            }
        }
    }
    Ok(())
}

fn ci() -> anyhow::Result<()> {
    let root = repo_root()?;
    run_in(
        root.clone(),
        "bash",
        &["scripts/verify-no-private-paths.sh"],
    )?;
    run_in(
        root.clone(),
        "bash",
        &[
            "scripts/security/scan-private-material.sh",
            "--git-tree",
            "--repo",
            ".",
        ],
    )?;
    run("cargo", &["fmt", "--all", "--", "--check"])?;
    run(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run("cargo", &["test", "-p", "exyonq-wasm-host"])?;
    run(
        "cargo",
        &["test", "--workspace", "--exclude", "exyonq-wasm-host"],
    )?;
    Ok(())
}

fn security(command: SecurityCommands) -> anyhow::Result<()> {
    let root = repo_root()?;
    match command {
        SecurityCommands::Check => security_check(&root)?,
        SecurityCommands::Audit { version } => security_audit(&root, &version)?,
        SecurityCommands::PreRelease { version } => {
            security_check(&root)?;
            run_in(
                root.clone(),
                "bash",
                &["scripts/legal/verify-release-legal-bundle.sh"],
            )?;
            security_verify_audit(&root, &version)?;
        }
    }
    Ok(())
}

fn security_check(root: &Path) -> anyhow::Result<()> {
    run_in(root, "cargo", &["audit"])?;
    run_in(root, "cargo", &["deny", "check"])?;
    Ok(())
}

fn deps(command: DepsCommands) -> anyhow::Result<()> {
    let root = repo_root()?;
    match command {
        DepsCommands::WasmtimeCheck => {
            run_in(root, "bash", &["scripts/deps/wasmtime-watch.sh"])?;
        }
    }
    Ok(())
}

fn security_audit(root: &Path, version: &str) -> anyhow::Result<()> {
    let template = root.join("docs/security/audit-template.md");
    let out = root.join(format!("docs/security/audit-v{version}.md"));
    if out.exists() {
        anyhow::bail!("audit file already exists: {}", out.display());
    }
    let body = std::fs::read_to_string(&template)
        .with_context(|| format!("read {}", template.display()))?;
    let commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "<commit>".to_string());
    let filled = body
        .replace("X.Y.Z", version)
        .replace("<full sha>", &commit)
        .replace("YYYY-MM-DD", &today_utc());
    std::fs::write(&out, filled)?;
    println!("wrote {}", out.display());
    Ok(())
}

fn security_verify_audit(root: &Path, version: &str) -> anyhow::Result<()> {
    run_in(root, "bash", &["scripts/verify-release-audit.sh", version])
}

fn today_utc() -> String {
    Command::new("date")
        .args(["-u", "+%Y-%m-%d"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "1970-01-01".to_string())
}

fn bench(command: BenchCommands) -> anyhow::Result<()> {
    let root = repo_root()?;
    match command {
        BenchCommands::Compare {
            functional_only,
            skip_up,
            duration,
            mode,
            tier,
            from_host,
            in_docker,
            include_roadmap,
            all_servers,
            no_cache,
            refresh_rivals,
        } => {
            let tier_arg = tier.to_string();
            let mut cmd = Command::new("cargo");
            cmd.args([
                "run",
                "-p",
                "exyonq-bench",
                "--",
                "compare",
                "--duration",
                duration.as_str(),
                "--mode",
                mode.as_str(),
                "--tier",
                tier_arg.as_str(),
            ]);
            if functional_only {
                cmd.arg("--functional-only");
            }
            if skip_up {
                cmd.arg("--skip-up");
            }
            if from_host {
                cmd.arg("--from-host");
            }
            if in_docker {
                cmd.arg("--in-docker");
            }
            if include_roadmap {
                cmd.arg("--include-roadmap");
            }
            if all_servers || no_cache {
                cmd.arg("--all-servers");
            }
            if no_cache {
                cmd.arg("--no-cache");
            }
            if refresh_rivals {
                cmd.arg("--refresh-rivals");
            }
            run_command(cmd, &root)?;
        }
        BenchCommands::CalibrateTiers { duration } => {
            run_in(
                root,
                "cargo",
                &[
                    "run",
                    "-p",
                    "exyonq-bench",
                    "--",
                    "calibrate-tiers",
                    "--duration",
                    &duration,
                ],
            )?;
        }
        BenchCommands::PeakScan {
            duration,
            warmup,
            peak_source,
            profile,
            point_duration_sec,
        } => {
            let mut cmd = Command::new("cargo");
            cmd.args([
                "run",
                "-p",
                "exyonq-bench",
                "--",
                "peak-scan",
                "--duration",
                duration.as_str(),
                "--warmup",
                warmup.as_str(),
            ]);
            if let Some(src) = peak_source {
                cmd.arg("--peak-source").arg(src);
            }
            if profile {
                cmd.arg("--profile");
            }
            if let Some(sec) = point_duration_sec {
                cmd.arg("--point-duration-sec").arg(sec.to_string());
            }
            run_command(cmd, &root)?;
        }
        BenchCommands::Functional => {
            run_in(
                root,
                "cargo",
                &["run", "-p", "exyonq-bench", "--", "functional"],
            )?;
        }
        BenchCommands::Perf {
            duration,
            mode,
            tier,
            from_host,
            in_docker,
            include_roadmap,
            all_servers,
            no_cache,
            refresh_rivals,
        } => {
            let tier_arg = tier.to_string();
            let mut cmd = Command::new("cargo");
            cmd.args([
                "run",
                "-p",
                "exyonq-bench",
                "--",
                "perf",
                "--duration",
                duration.as_str(),
                "--mode",
                mode.as_str(),
                "--tier",
                tier_arg.as_str(),
            ]);
            if from_host {
                cmd.arg("--from-host");
            }
            if in_docker {
                cmd.arg("--in-docker");
            }
            if include_roadmap {
                cmd.arg("--include-roadmap");
            }
            if all_servers || no_cache {
                cmd.arg("--all-servers");
            }
            if no_cache {
                cmd.arg("--no-cache");
            }
            if refresh_rivals {
                cmd.arg("--refresh-rivals");
            }
            run_command(cmd, &root)?;
        }
        BenchCommands::RivalResources { duration } => {
            let mut cmd = Command::new("bash");
            cmd.arg("benchmarks/scenarios/perf/refresh-rivals-resources.sh")
                .env("BENCH_DURATION", &duration)
                .env("BENCH_RESULTS_DIR", root.join("benchmarks/results/latest"))
                .env(
                    "BENCH_COMPOSE_FILE",
                    root.join("benchmarks/docker/docker-compose.bench.yml"),
                )
                .env("BENCH_COMPOSE_DIR", root.join("benchmarks/docker"));
            run_command(cmd, &root)?;
        }
    }
    Ok(())
}

fn run_command(mut cmd: Command, cwd: &Path) -> anyhow::Result<()> {
    let status = cmd
        .current_dir(cwd)
        .status()
        .with_context(|| "failed to run benchmark command")?;
    if !status.success() {
        anyhow::bail!("benchmark command failed");
    }
    Ok(())
}

fn repo_root() -> anyhow::Result<PathBuf> {
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask parent")?
        .to_path_buf())
}

fn run_in(cwd: impl AsRef<Path>, cmd: &str, args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new(cmd)
        .args(args)
        .current_dir(cwd.as_ref())
        .status()
        .with_context(|| format!("failed to run {cmd}"))?;
    if !status.success() {
        anyhow::bail!("{cmd} failed");
    }
    Ok(())
}

fn run(cmd: &str, args: &[&str]) -> anyhow::Result<()> {
    run_in(repo_root()?, cmd, args)
}
