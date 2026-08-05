use super::compare::{
    build_compare_report_with_mode, compare_median_to_baseline, merge_verdicts, Verdict,
};
use super::normalize::{normalize_from_summary_json, normalize_raw_rewrk_json, read_summary_file};
use super::registry;
use super::scenarios::{
    fingerprint_for_scenario, full_profile, load_contract_scenarios, quick_profile,
    ContractScenario,
};
use super::schema::ProfileSpec;
use super::schema::{
    read_baseline, write_baseline, write_run, BaselineDocument, BaselineScenarioEntry, RunDocument,
    CONTRACT_ID, CONTRACT_SCHEMA_VERSION,
};
use anyhow::Context;
use clap::Args;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Args, Debug)]
pub struct ContractArgs {
    /// Quick profile (5s measure, 3 repetitions) — local gate.
    #[arg(long, conflicts_with = "full")]
    quick: bool,
    /// Full profile definition (30s) — prepared but not required to execute here.
    #[arg(long, conflicts_with = "quick")]
    full: bool,
    /// Execute ExyonQ-only load (requires Linux/Docker bench stack).
    #[arg(long)]
    run: bool,
    /// Write baseline from current run samples (never implicit).
    #[arg(long, conflicts_with = "accept_unvalidated_seed")]
    accept_baseline: bool,
    /// Administrative seed write without gate validation (`status=seeded_reference`, `official=false`).
    #[arg(long, conflicts_with = "accept_baseline")]
    accept_unvalidated_seed: bool,
    /// Gate leg rc values from wrapper — block `--accept-baseline` when any is non-zero.
    #[arg(long, hide = true)]
    after_quick_rc: Option<i32>,
    #[arg(long, hide = true)]
    after_full_rc: Option<i32>,
    #[arg(long, hide = true)]
    after_cleanup_rc: Option<i32>,
    #[arg(long)]
    baseline: Option<PathBuf>,
    #[arg(long)]
    compare: Option<PathBuf>,
    #[arg(long)]
    scenario: Option<String>,
    #[arg(long)]
    json_output: Option<PathBuf>,
    /// Directory with per-scenario run JSON files (`P1-rep1.json`, …) or summary tree.
    #[arg(long)]
    samples_dir: Option<PathBuf>,
    /// Quick-profile samples when accepting a full baseline (`--quick-samples-dir`).
    #[arg(long)]
    quick_samples_dir: Option<PathBuf>,
}

pub fn contract_command(repo_root: &Path, args: ContractArgs) -> anyhow::Result<i32> {
    let baseline_path = match args.baseline.clone() {
        Some(p) => p,
        None => registry::resolve_baseline_path(repo_root)?,
    };
    let profile = if args.full {
        full_profile()
    } else {
        quick_profile()
    };

    if args.accept_baseline || args.accept_unvalidated_seed {
        return accept_baseline(
            repo_root,
            &baseline_path,
            &profile,
            args.samples_dir.as_deref(),
            args.quick_samples_dir.as_deref(),
            args.accept_unvalidated_seed,
            args.after_quick_rc,
            args.after_full_rc,
            args.after_cleanup_rc,
        );
    }

    if args.run {
        return run_quick_and_gate(
            repo_root,
            &baseline_path,
            &profile,
            args.json_output.as_deref(),
        );
    }

    if let Some(compare_path) = args.compare.as_ref() {
        return compare_file_to_baseline(&baseline_path, compare_path, args.scenario.as_deref());
    }

    if let Some(samples) = args.samples_dir.as_ref() {
        let outcome = compare_samples_dir(
            repo_root,
            &baseline_path,
            samples,
            &profile,
            args.scenario.as_deref(),
        )?;
        if args.json_output.is_some() || !outcome.lines.is_empty() {
            let baseline = read_baseline(&baseline_path)?;
            print_report(
                &baseline,
                &outcome.lines,
                outcome.mode,
                outcome.collection_status,
            );
            if let Some(path) = args.json_output.as_ref() {
                let report = build_compare_report_with_mode(
                    &baseline,
                    outcome.lines.clone(),
                    outcome.mode,
                    outcome.collection_status,
                );
                let json = serde_json::to_string_pretty(&report)?;
                std::fs::write(path, format!("{json}\n"))?;
            }
        }
        return Ok(outcome.exit_code);
    }

    // Default: validate comparator + baseline readability.
    let baseline = read_baseline(&baseline_path)
        .with_context(|| format!("read baseline {}", baseline_path.display()))?;
    println!(
        "performance contract {} loaded ({} scenarios, status={})",
        baseline.contract_id,
        baseline.scenarios.len(),
        baseline.status
    );
    println!("use --samples-dir, --compare, or --run --quick");
    Ok(0)
}

fn host_meta() -> (String, String, u32) {
    let arch = std::env::consts::ARCH.to_string();
    let os = std::env::consts::OS.to_string();
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    (os, arch, cpus)
}

fn git_commit(repo_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if output.status.success() {
        return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let stamp = repo_root.join(".baseline-source-commit");
    std::fs::read_to_string(stamp)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn scenario_by_id<'a>(
    scenarios: &'a [ContractScenario],
    id: &str,
) -> anyhow::Result<&'a ContractScenario> {
    scenarios
        .iter()
        .find(|s| s.id.eq_ignore_ascii_case(id))
        .with_context(|| format!("unknown scenario {id}"))
}

fn load_run_json(
    path: &Path,
    scenario: &ContractScenario,
    profile: &ProfileSpec,
) -> anyhow::Result<RunDocument> {
    if path.extension().and_then(|s| s.to_str()) == Some("json") {
        let value = read_summary_file(path)?;
        if value.get("schema_version").is_some() && value.get("fingerprint").is_some() {
            return Ok(serde_json::from_value(value)?);
        }
        let (platform, arch, cpus) = host_meta();
        return Ok(normalize_from_summary_json(
            &value, scenario, profile, &platform, &arch, cpus, None,
        )?
        .doc);
    }
    anyhow::bail!("unsupported run file {}", path.display());
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunDirProfile {
    Quick,
    Full,
}

pub fn run_dir_prefix(profile: &ProfileSpec) -> &'static str {
    if profile.duration_measure == full_profile().duration_measure {
        "perf-contract-full"
    } else {
        "perf-contract-quick"
    }
}

pub fn run_dir_profile_from_name(name: &str) -> Option<RunDirProfile> {
    if name.starts_with("perf-contract-full-") {
        Some(RunDirProfile::Full)
    } else if name.starts_with("perf-contract-quick-") {
        Some(RunDirProfile::Quick)
    } else {
        None
    }
}

/// First-seed full policy: collect-only when seeded_reference baseline has no full-duration entries.
pub fn seeded_reference_collect_only(baseline: &BaselineDocument, profile: &ProfileSpec) -> bool {
    if baseline.status != "seeded_reference" {
        return false;
    }
    if profile.duration_measure != full_profile().duration_measure {
        return false;
    }
    baseline
        .scenarios
        .values()
        .all(|entry| entry.fingerprint.duration_measure != profile.duration_measure)
}

struct CompareOutcome {
    exit_code: i32,
    lines: Vec<(String, Verdict)>,
    mode: Option<&'static str>,
    collection_status: Option<&'static str>,
}

fn validate_samples_in_dir(
    samples_dir: &Path,
    profile: &ProfileSpec,
    scenarios: &[ContractScenario],
) -> CompareOutcome {
    let mut lines = Vec::new();
    let mut invalid = Vec::new();
    for scenario in scenarios {
        let mut runs = Vec::new();
        for rep in 1..=profile.repetitions {
            let path = samples_dir.join(format!("{}-rep{rep}.json", scenario.id));
            if !path.exists() {
                invalid.push(format!("missing {}", path.display()));
                continue;
            }
            match load_run_json(&path, scenario, profile) {
                Ok(doc) => runs.push(doc),
                Err(e) => invalid.push(format!("{}: {e}", path.display())),
            }
        }
        if runs.len() != profile.repetitions as usize {
            invalid.push(format!(
                "{}: expected {} reps, found {}",
                scenario.id,
                profile.repetitions,
                runs.len()
            ));
        } else if runs.iter().any(|r| !r.valid) {
            invalid.push(format!("{}: invalid sample(s)", scenario.id));
        } else {
            lines.push((scenario.id.clone(), Verdict::Pass));
        }
    }
    if !invalid.is_empty() {
        CompareOutcome {
            exit_code: Verdict::InvalidResult(invalid.clone()).exit_code(),
            lines: scenarios
                .iter()
                .map(|s| (s.id.clone(), Verdict::InvalidResult(invalid.clone())))
                .collect(),
            mode: Some("collect_only"),
            collection_status: None,
        }
    } else {
        CompareOutcome {
            exit_code: 0,
            lines,
            mode: Some("collect_only"),
            collection_status: Some("VALID"),
        }
    }
}

fn compare_samples_dir(
    repo_root: &Path,
    baseline_path: &Path,
    samples_dir: &Path,
    profile: &ProfileSpec,
    scenario_filter: Option<&str>,
) -> anyhow::Result<CompareOutcome> {
    let scenarios = load_contract_scenarios(repo_root)?;
    let baseline = read_baseline(baseline_path)?;
    if seeded_reference_collect_only(&baseline, profile) {
        return Ok(validate_samples_in_dir(samples_dir, profile, &scenarios));
    }
    let mut lines = Vec::new();

    for scenario in &scenarios {
        if let Some(filter) = scenario_filter {
            if !scenario.id.eq_ignore_ascii_case(filter) {
                continue;
            }
        }
        let entry = baseline_entry_for_profile(&baseline, &scenario.id, profile)
            .with_context(|| format!("baseline missing scenario {}", scenario.id))?;
        let gate = baseline
            .scenario_gates
            .as_ref()
            .and_then(|g| g.get(&scenario.id));
        let mut runs = Vec::new();
        for rep in 1..=profile.repetitions {
            let path = samples_dir.join(format!("{}-rep{rep}.json", scenario.id));
            if !path.exists() {
                let alt = samples_dir.join(format!("{}.json", scenario.id.to_ascii_lowercase()));
                if rep == 1 && alt.exists() {
                    runs.push(load_run_json(&alt, scenario, profile)?);
                }
                continue;
            }
            runs.push(load_run_json(&path, scenario, profile)?);
        }
        if runs.is_empty() {
            lines.push((
                scenario.id.clone(),
                Verdict::InvalidResult(vec![format!("no samples in {}", samples_dir.display())]),
            ));
            continue;
        }
        let verdict = if runs.len() == 1 {
            super::compare::compare_run_to_baseline(entry, &runs[0], gate)
        } else {
            compare_median_to_baseline(entry, &runs, gate)
        };
        lines.push((scenario.id.clone(), verdict));
    }

    let exit_code =
        merge_verdicts(&lines.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>()).exit_code();
    Ok(CompareOutcome {
        exit_code,
        lines,
        mode: None,
        collection_status: None,
    })
}

fn compare_file_to_baseline(
    baseline_path: &Path,
    compare_path: &Path,
    scenario_filter: Option<&str>,
) -> anyhow::Result<i32> {
    let baseline = read_baseline(baseline_path)?;
    let run: RunDocument = serde_json::from_str(&std::fs::read_to_string(compare_path)?)?;
    let scenario = scenario_filter
        .map(|s| s.to_ascii_uppercase())
        .unwrap_or_else(|| run.fingerprint.scenario.clone());
    let entry = baseline
        .scenarios
        .get(&scenario)
        .with_context(|| format!("baseline missing {scenario}"))?;
    let gate = baseline
        .scenario_gates
        .as_ref()
        .and_then(|g| g.get(&scenario));
    let verdict = super::compare::compare_run_to_baseline(entry, &run, gate);
    let lines = vec![(scenario.clone(), verdict.clone())];
    print_report(&baseline, &lines, None, None);
    Ok(verdict.exit_code())
}

fn baseline_entry_for_profile<'a>(
    baseline: &'a BaselineDocument,
    scenario_id: &str,
    profile: &ProfileSpec,
) -> Option<&'a BaselineScenarioEntry> {
    if profile.duration_measure == quick_profile().duration_measure {
        if let Some(q) = baseline.quick_scenarios.as_ref() {
            if let Some(e) = q.get(scenario_id) {
                return Some(e);
            }
        }
    }
    baseline.scenarios.get(scenario_id)
}

fn accept_baseline(
    repo_root: &Path,
    baseline_path: &Path,
    profile: &ProfileSpec,
    samples_dir: Option<&Path>,
    quick_samples_dir: Option<&Path>,
    unvalidated_seed: bool,
    after_quick_rc: Option<i32>,
    after_full_rc: Option<i32>,
    after_cleanup_rc: Option<i32>,
) -> anyhow::Result<i32> {
    if !unvalidated_seed {
        for (label, rc) in [
            ("quick_rc", after_quick_rc),
            ("full_rc", after_full_rc),
            ("cleanup_rc", after_cleanup_rc),
        ] {
            if let Some(rc) = rc {
                if rc != 0 {
                    anyhow::bail!(
                        "--accept-baseline blocked: {label}={rc} (use --accept-unvalidated-seed for administrative seed only)"
                    );
                }
            }
        }
    }

    let samples_dir =
        samples_dir.context("--accept-baseline requires --samples-dir with run JSON samples")?;
    let scenarios = load_contract_scenarios(repo_root)?;
    let (platform, arch, cpus) = host_meta();
    let mut scenario_entries = BTreeMap::new();

    for scenario in &scenarios {
        let mut runs = Vec::new();
        for rep in 1..=profile.repetitions {
            let path = samples_dir.join(format!("{}-rep{rep}.json", scenario.id));
            if path.exists() {
                runs.push(load_run_json(&path, scenario, profile)?);
            }
        }
        if runs.is_empty() {
            anyhow::bail!("missing samples for {}", scenario.id);
        }
        let fingerprint = fingerprint_for_scenario(scenario, profile, &platform, &arch, cpus);
        let metrics = super::compare::median_metrics(
            &runs.iter().map(|r| r.metrics.clone()).collect::<Vec<_>>(),
        )
        .context("median")?;
        scenario_entries.insert(
            scenario.id.clone(),
            BaselineScenarioEntry {
                fingerprint,
                metrics,
                repetitions: runs.len() as u32,
                notes: Some("accepted via xtask perf contract --accept-baseline".into()),
            },
        );
    }

    let mut quick_entries = BTreeMap::new();
    if let Some(qdir) = quick_samples_dir {
        let qprofile = quick_profile();
        for scenario in &scenarios {
            let mut runs = Vec::new();
            for rep in 1..=qprofile.repetitions {
                let path = qdir.join(format!("{}-rep{rep}.json", scenario.id));
                if path.exists() {
                    runs.push(load_run_json(&path, scenario, &qprofile)?);
                }
            }
            if runs.is_empty() {
                anyhow::bail!("missing quick samples for {}", scenario.id);
            }
            let fingerprint = fingerprint_for_scenario(scenario, &qprofile, &platform, &arch, cpus);
            let metrics = super::compare::median_metrics(
                &runs.iter().map(|r| r.metrics.clone()).collect::<Vec<_>>(),
            )
            .context("quick median")?;
            quick_entries.insert(
                scenario.id.clone(),
                BaselineScenarioEntry {
                    fingerprint,
                    metrics,
                    repetitions: runs.len() as u32,
                    notes: Some("quick profile via --quick-samples-dir".into()),
                },
            );
        }
    }

    let doc = BaselineDocument {
        schema_version: CONTRACT_SCHEMA_VERSION,
        contract_id: CONTRACT_ID.into(),
        status: if unvalidated_seed {
            "seeded_reference".into()
        } else {
            "accepted".into()
        },
        created_at: format!("unix:{}", unix_now()),
        commit: git_commit(repo_root),
        host_class: format!("{platform}-{arch}"),
        quick_profile: Some(quick_profile()),
        full_profile: full_profile(),
        scenarios: scenario_entries,
        quick_scenarios: if quick_entries.is_empty() {
            None
        } else {
            Some(quick_entries)
        },
        official: if unvalidated_seed { Some(false) } else { None },
        approval_status: None,
        baseline_type: None,
        worker_mode: None,
        host_class_netcup: None,
        scenario_gates: None,
    };
    write_baseline(baseline_path, &doc)?;
    println!("baseline written to {}", baseline_path.display());
    Ok(0)
}

fn run_quick_and_gate(
    repo_root: &Path,
    baseline_path: &Path,
    profile: &ProfileSpec,
    json_output: Option<&Path>,
) -> anyhow::Result<i32> {
    if !bench_stack_available() {
        println!("SKIP live quick run: bench stack unavailable (need Linux+Docker+rewrk via exyonq-bench)");
        println!("comparator and baseline gate remain available via --samples-dir");
        return Ok(0);
    }

    let out_dir = repo_root.join(format!(
        "benchmarks/results/{}-{}",
        run_dir_prefix(profile),
        unix_now()
    ));
    std::fs::create_dir_all(&out_dir)?;

    for scenario in load_contract_scenarios(repo_root)? {
        for rep in 1..=profile.repetitions {
            run_one_scenario(repo_root, &out_dir, &scenario, profile, rep)?;
        }
    }

    let outcome = compare_samples_dir(repo_root, baseline_path, &out_dir, profile, None)?;
    print_report(
        &read_baseline(baseline_path)?,
        &outcome.lines,
        outcome.mode,
        outcome.collection_status,
    );
    if let Some(path) = json_output {
        let baseline = read_baseline(baseline_path)?;
        let report = build_compare_report_with_mode(
            &baseline,
            outcome.lines.clone(),
            outcome.mode,
            outcome.collection_status,
        );
        let json = serde_json::to_string_pretty(&report)?;
        std::fs::write(path, format!("{json}\n"))?;
    }
    Ok(outcome.exit_code)
}

fn bench_stack_available() -> bool {
    if std::env::consts::OS != "linux" {
        return false;
    }
    Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn warmup_sec(profile: &ProfileSpec) -> String {
    let d = profile.duration_warmup.as_str();
    if let Some(n) = d.strip_suffix('s') {
        return n.to_string();
    }
    if let Some(n) = d.strip_suffix('m') {
        return (n.parse::<u32>().unwrap_or(0) * 60).to_string();
    }
    "3".into()
}

fn run_one_scenario(
    repo_root: &Path,
    out_dir: &Path,
    scenario: &ContractScenario,
    profile: &ProfileSpec,
    rep: u32,
) -> anyhow::Result<()> {
    let filter = scenario.id.to_ascii_lowercase();
    let status = Command::new("cargo")
        .args([
            "run",
            "-p",
            "exyonq-bench",
            "--",
            "perf",
            "--duration",
            profile.duration_measure.as_str(),
            "--in-docker",
        ])
        .env("BENCH_SCENARIOS_FILTER", &filter)
        .env("BENCH_EXYONQ_ONLY", "1")
        .env("BENCH_SKIP_FUNCTIONAL", "1")
        .env("BENCH_SKIP_UP", "1")
        .env(
            "COMPOSE_PROJECT_NAME",
            std::env::var("COMPOSE_PROJECT_NAME").unwrap_or_else(|_| "exyonq-perf-contract".into()),
        )
        .env("BENCH_WARMUP_SEC", warmup_sec(profile))
        .env("BENCH_RESULTS_DIR", out_dir)
        .current_dir(repo_root)
        .status()
        .context("exyonq-bench perf")?;
    if !status.success() {
        anyhow::bail!("bench failed for {} rep {rep}", scenario.id);
    }

    let summary_path = out_dir.join(format!("{filter}/exyonq/summary.json"));
    let raw_path = out_dir.join(format!("{filter}-exyonq.json"));
    let summary_value = if summary_path.exists() {
        read_summary_file(&summary_path)?
    } else if raw_path.exists() {
        read_summary_file(&raw_path)?
    } else {
        anyhow::bail!("missing bench output for {}", scenario.id);
    };

    let (platform, arch, cpus) = host_meta();
    let run = if summary_value.get("rps").is_some() {
        normalize_from_summary_json(
            &summary_value,
            scenario,
            profile,
            &platform,
            &arch,
            cpus,
            git_commit(repo_root),
        )?
    } else {
        normalize_raw_rewrk_json(
            &summary_value,
            scenario,
            profile,
            &platform,
            &arch,
            cpus,
            git_commit(repo_root),
        )?
    };
    let out = out_dir.join(format!("{}-rep{rep}.json", scenario.id));
    write_run(&out, &run.doc)?;
    Ok(())
}

fn print_report(
    baseline: &BaselineDocument,
    lines: &[(String, Verdict)],
    mode: Option<&str>,
    collection_status: Option<&str>,
) {
    let report = build_compare_report_with_mode(baseline, lines.to_vec(), mode, collection_status);
    println!("overall: {}", report.overall_verdict);
    if let Some(status) = report.collection_status.as_ref() {
        println!("collection_status: {status}");
    }
    for line in &report.scenarios {
        println!("  {}: {}", line.scenario, line.verdict);
        for msg in &line.messages {
            println!("    - {msg}");
        }
    }
}

fn unix_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perf::contract::compare::{build_compare_report, build_compare_report_with_mode};
    use crate::perf::contract::scenarios::quick_profile;
    use crate::perf::contract::schema::{write_run, BaselineScenarioEntry, MetricsSnapshot};

    fn write_scenarios_toml(repo: &Path) {
        std::fs::create_dir_all(repo.join("benchmarks/scenarios/perf")).expect("mkdir");
        std::fs::write(
            repo.join("benchmarks/scenarios/perf/scenarios.toml"),
            include_str!("../../../fixtures/scenarios.toml"),
        )
        .expect("scenarios");
    }

    fn sample_run(_repo: &Path, scenario: &ContractScenario) -> RunDocument {
        let (platform, arch, cpus) = host_meta();
        normalize_from_summary_json(
            &serde_json::json!({
                "rps": 1000.0,
                "p50_ms": 1.0,
                "p95_ms": 2.0,
                "p99_ms": 3.0,
                "success_rate": 100.0,
                "valid": true
            }),
            scenario,
            &quick_profile(),
            &platform,
            &arch,
            cpus,
            None,
        )
        .expect("normalize")
        .doc
    }

    #[test]
    fn accept_baseline_requires_explicit_flag_and_samples() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path();
        write_scenarios_toml(repo);
        let samples = dir.path().join("samples");
        std::fs::create_dir_all(&samples).expect("samples");
        let baseline_path = dir.path().join("baseline.json");

        let err = accept_baseline(
            repo,
            &baseline_path,
            &quick_profile(),
            None,
            None,
            false,
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("--samples-dir"));

        let scenario = load_contract_scenarios(repo).expect("scenarios")[0].clone();
        let run = sample_run(repo, &scenario);
        for id in ["P1", "P2", "P4", "P7"] {
            for rep in 1..=3 {
                let path = samples.join(format!("{id}-rep{rep}.json"));
                write_run(&path, &run).expect("write");
            }
        }
        accept_baseline(
            repo,
            &baseline_path,
            &quick_profile(),
            Some(&samples),
            None,
            false,
            None,
            None,
            None,
        )
        .expect("accept");
        assert!(baseline_path.exists());
        let saved = std::fs::read_to_string(&baseline_path).expect("read");
        assert!(saved.contains("performance-contract-v1"));
        assert!(saved.contains("\"status\": \"accepted\""));
    }

    #[test]
    fn accept_baseline_blocked_after_gate_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path();
        write_scenarios_toml(repo);
        let samples = repo.join("samples");
        std::fs::create_dir_all(&samples).expect("samples");
        let baseline_path = repo.join("baseline.json");
        let scenario = load_contract_scenarios(repo).expect("scenarios")[0].clone();
        let run = sample_run(repo, &scenario);
        for id in ["P1", "P2", "P4", "P7"] {
            for rep in 1..=3 {
                write_run(&samples.join(format!("{id}-rep{rep}.json")), &run).expect("write");
            }
        }
        let err = accept_baseline(
            repo,
            &baseline_path,
            &quick_profile(),
            Some(&samples),
            None,
            false,
            Some(1),
            Some(3),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("blocked"));
    }

    #[test]
    fn accept_unvalidated_seed_sets_seeded_reference() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path();
        write_scenarios_toml(repo);
        let samples = repo.join("samples");
        std::fs::create_dir_all(&samples).expect("samples");
        let baseline_path = repo.join("baseline.json");
        let scenario = load_contract_scenarios(repo).expect("scenarios")[0].clone();
        let run = sample_run(repo, &scenario);
        for id in ["P1", "P2", "P4", "P7"] {
            for rep in 1..=3 {
                write_run(&samples.join(format!("{id}-rep{rep}.json")), &run).expect("write");
            }
        }
        accept_baseline(
            repo,
            &baseline_path,
            &quick_profile(),
            Some(&samples),
            None,
            true,
            Some(1),
            Some(3),
            None,
        )
        .expect("unvalidated seed");
        let saved = std::fs::read_to_string(&baseline_path).expect("read");
        assert!(saved.contains("\"status\": \"seeded_reference\""));
        assert!(saved.contains("\"official\": false"));
    }

    #[test]
    fn run_dir_prefix_quick_vs_full() {
        assert_eq!(run_dir_prefix(&quick_profile()), "perf-contract-quick");
        assert_eq!(run_dir_prefix(&full_profile()), "perf-contract-full");
    }

    #[test]
    fn run_dir_profile_from_name_separates_paths() {
        assert_eq!(
            run_dir_profile_from_name("perf-contract-quick-123"),
            Some(RunDirProfile::Quick)
        );
        assert_eq!(
            run_dir_profile_from_name("perf-contract-full-456"),
            Some(RunDirProfile::Full)
        );
        assert_eq!(run_dir_profile_from_name("perf-contract-legacy"), None);
    }

    #[test]
    fn seeded_reference_collect_only_when_no_full_fingerprints() {
        let baseline = BaselineDocument {
            schema_version: 1,
            contract_id: CONTRACT_ID.into(),
            status: "seeded_reference".into(),
            created_at: "0".into(),
            commit: None,
            host_class: "linux-aarch64".into(),
            quick_profile: Some(quick_profile()),
            full_profile: full_profile(),
            scenarios: BTreeMap::from([(
                "P1".into(),
                BaselineScenarioEntry {
                    fingerprint: fingerprint_for_scenario(
                        &ContractScenario {
                            id: "P1".into(),
                            label: "P1".into(),
                            path: "/site/1k.bin".into(),
                            connections: 100,
                            duration: "5s".into(),
                            scheme: "http".into(),
                            features: vec![],
                            payload_label: "1KiB".into(),
                        },
                        &quick_profile(),
                        "linux",
                        "aarch64",
                        4,
                    ),
                    metrics: MetricsSnapshot::default(),
                    repetitions: 3,
                    notes: None,
                },
            )]),
            quick_scenarios: None,
            official: None,
            approval_status: None,
            baseline_type: None,
            worker_mode: None,
            host_class_netcup: None,
            scenario_gates: None,
        };
        assert!(seeded_reference_collect_only(&baseline, &full_profile()));
        assert!(!seeded_reference_collect_only(&baseline, &quick_profile()));
    }

    #[test]
    fn full_seed_reference_when_fingerprints_match() {
        let scenario = ContractScenario {
            id: "P1".into(),
            label: "P1".into(),
            path: "/site/1k.bin".into(),
            connections: 100,
            duration: "30s".into(),
            scheme: "http".into(),
            features: vec![],
            payload_label: "1KiB".into(),
        };
        let baseline = BaselineDocument {
            schema_version: 1,
            contract_id: CONTRACT_ID.into(),
            status: "seeded_reference".into(),
            created_at: "0".into(),
            commit: None,
            host_class: "linux-aarch64".into(),
            quick_profile: Some(quick_profile()),
            full_profile: full_profile(),
            scenarios: BTreeMap::from([(
                "P1".into(),
                BaselineScenarioEntry {
                    fingerprint: fingerprint_for_scenario(
                        &scenario,
                        &full_profile(),
                        "linux",
                        "aarch64",
                        4,
                    ),
                    metrics: MetricsSnapshot::default(),
                    repetitions: 5,
                    notes: None,
                },
            )]),
            quick_scenarios: None,
            official: None,
            approval_status: None,
            baseline_type: None,
            worker_mode: None,
            host_class_netcup: None,
            scenario_gates: None,
        };
        assert!(!seeded_reference_collect_only(&baseline, &full_profile()));
    }

    #[test]
    fn full_mismatch_5s_30s_rejected_before_compare() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path();
        write_scenarios_toml(repo);
        let baseline_path = repo.join("baseline.json");
        let scenario = load_contract_scenarios(repo).expect("scenarios")[0].clone();
        let baseline = BaselineDocument {
            schema_version: 1,
            contract_id: CONTRACT_ID.into(),
            status: "seeded_reference".into(),
            created_at: "0".into(),
            commit: None,
            host_class: "linux-aarch64".into(),
            quick_profile: Some(quick_profile()),
            full_profile: full_profile(),
            scenarios: BTreeMap::from([(
                scenario.id.clone(),
                BaselineScenarioEntry {
                    fingerprint: fingerprint_for_scenario(
                        &scenario,
                        &quick_profile(),
                        "linux",
                        "aarch64",
                        4,
                    ),
                    metrics: MetricsSnapshot {
                        requests_per_second: 1000.0,
                        p50_ms: 1.0,
                        p95_ms: 2.0,
                        p99_ms: 3.0,
                        error_count: 0,
                        error_rate: 0.0,
                        unexpected_5xx: 0,
                        process_rss_bytes: None,
                    },
                    repetitions: 3,
                    notes: None,
                },
            )]),
            quick_scenarios: None,
            official: None,
            approval_status: None,
            baseline_type: None,
            worker_mode: None,
            host_class_netcup: None,
            scenario_gates: None,
        };
        write_baseline(&baseline_path, &baseline).expect("write baseline");

        let samples = repo.join("full-samples");
        std::fs::create_dir_all(&samples).expect("samples");
        let (platform, arch, cpus) = host_meta();
        let full_run = normalize_from_summary_json(
            &serde_json::json!({
                "rps": 1000.0,
                "p50_ms": 1.0,
                "p95_ms": 2.0,
                "p99_ms": 3.0,
                "success_rate": 100.0,
                "valid": true
            }),
            &scenario,
            &full_profile(),
            &platform,
            &arch,
            cpus,
            None,
        )
        .expect("normalize");
        for id in ["P1", "P2", "P4", "P7"] {
            for rep in 1..=5 {
                write_run(&samples.join(format!("{id}-rep{rep}.json")), &full_run.doc)
                    .expect("write");
            }
        }

        let outcome = compare_samples_dir(repo, &baseline_path, &samples, &full_profile(), None)
            .expect("compare");
        assert_eq!(outcome.exit_code, 0);
        assert_eq!(outcome.mode, Some("collect_only"));
        assert_eq!(outcome.collection_status, Some("VALID"));
        assert_eq!(outcome.lines.len(), 4);
    }

    #[test]
    fn json_output_matches_stdout_verdict() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path();
        write_scenarios_toml(repo);
        let baseline_path = repo.join("baseline.json");
        let samples = repo.join("samples");
        std::fs::create_dir_all(&samples).expect("samples");
        let scenarios = load_contract_scenarios(repo).expect("scenarios");
        let (platform, arch, cpus) = host_meta();
        let mut scenario_entries = BTreeMap::new();
        for scenario in &scenarios {
            let fp = fingerprint_for_scenario(scenario, &quick_profile(), &platform, &arch, cpus);
            let run = sample_run(repo, scenario);
            for rep in 1..=3 {
                write_run(
                    &samples.join(format!("{}-rep{rep}.json", scenario.id)),
                    &run,
                )
                .expect("write");
            }
            scenario_entries.insert(
                scenario.id.clone(),
                BaselineScenarioEntry {
                    fingerprint: fp,
                    metrics: run.metrics.clone(),
                    repetitions: 3,
                    notes: None,
                },
            );
        }
        let baseline = BaselineDocument {
            schema_version: 1,
            contract_id: CONTRACT_ID.into(),
            status: "accepted".into(),
            created_at: "0".into(),
            commit: None,
            host_class: format!("{platform}-{arch}"),
            quick_profile: Some(quick_profile()),
            full_profile: full_profile(),
            scenarios: scenario_entries,
            quick_scenarios: None,
            official: None,
            approval_status: None,
            baseline_type: None,
            worker_mode: None,
            host_class_netcup: None,
            scenario_gates: None,
        };
        write_baseline(&baseline_path, &baseline).expect("baseline");

        let outcome = compare_samples_dir(repo, &baseline_path, &samples, &quick_profile(), None)
            .expect("compare");
        let report = build_compare_report_with_mode(
            &baseline,
            outcome.lines.clone(),
            outcome.mode,
            outcome.collection_status,
        );
        assert!(!report.scenarios.is_empty());
        assert_eq!(report.overall_verdict, "PASS");
        assert_eq!(report.scenarios.len(), 4);
        for line in &report.scenarios {
            assert_eq!(line.verdict, "PASS");
        }
    }

    #[test]
    fn empty_scenarios_compare_is_invalid() {
        let baseline = BaselineDocument {
            schema_version: 1,
            contract_id: CONTRACT_ID.into(),
            status: "accepted".into(),
            created_at: "0".into(),
            commit: None,
            host_class: "linux-aarch64".into(),
            quick_profile: Some(quick_profile()),
            full_profile: full_profile(),
            scenarios: BTreeMap::new(),
            quick_scenarios: None,
            official: None,
            approval_status: None,
            baseline_type: None,
            worker_mode: None,
            host_class_netcup: None,
            scenario_gates: None,
        };
        let report = build_compare_report(&baseline, vec![]);
        assert_eq!(report.overall_verdict, "INVALID_RESULT");
        assert!(report.scenarios.is_empty());
    }
}
