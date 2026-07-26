use super::schema::{
    BaselineDocument, BaselineScenarioEntry, CompareReport, Fingerprint, MetricsSnapshot,
    RunDocument, ScenarioCompareLine, ScenarioGatePolicy, ToleranceBands, CONTRACT_ID,
    CONTRACT_SCHEMA_VERSION,
};
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Warning(Vec<String>),
    RegressionFail(Vec<String>),
    InvalidResult(Vec<String>),
    NotComparable(Vec<String>),
}

impl Verdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::Warning(_) => "WARNING",
            Verdict::RegressionFail(_) => "REGRESSION_FAIL",
            Verdict::InvalidResult(_) => "INVALID_RESULT",
            Verdict::NotComparable(_) => "NOT_COMPARABLE",
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Verdict::Pass | Verdict::Warning(_) => 0,
            Verdict::RegressionFail(_) => 1,
            Verdict::InvalidResult(_) => 2,
            Verdict::NotComparable(_) => 3,
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Verdict::Pass | Verdict::Warning(_))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Tolerances {
    pub rps_warn_pct: f64,
    pub rps_fail_pct: f64,
    pub p99_warn_pct: f64,
    pub p99_fail_pct: f64,
}

impl Default for Tolerances {
    fn default() -> Self {
        Self {
            rps_warn_pct: -5.0,
            rps_fail_pct: -10.0,
            p99_warn_pct: 10.0,
            p99_fail_pct: 20.0,
        }
    }
}

impl Tolerances {
    pub fn from_bands(bands: &ToleranceBands) -> Self {
        Self {
            rps_warn_pct: bands.rps_warn_pct,
            rps_fail_pct: bands.rps_fail_pct,
            p99_warn_pct: bands.p99_warn_pct,
            p99_fail_pct: bands.p99_fail_pct,
        }
    }

    pub fn extreme_default() -> Self {
        Self {
            rps_warn_pct: -5.0,
            rps_fail_pct: -25.0,
            p99_warn_pct: 10.0,
            p99_fail_pct: 50.0,
        }
    }
}

pub fn median_metrics(samples: &[MetricsSnapshot]) -> Option<MetricsSnapshot> {
    if samples.is_empty() {
        return None;
    }
    let mut rps: Vec<f64> = samples.iter().map(|m| m.requests_per_second).collect();
    let mut p50: Vec<f64> = samples.iter().map(|m| m.p50_ms).collect();
    let mut p95: Vec<f64> = samples.iter().map(|m| m.p95_ms).collect();
    let mut p99: Vec<f64> = samples.iter().map(|m| m.p99_ms).collect();
    rps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    p50.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    p95.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    p99.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = samples.len() / 2;
    Some(MetricsSnapshot {
        requests_per_second: rps[mid],
        p50_ms: p50[mid],
        p95_ms: p95[mid],
        p99_ms: p99[mid],
        error_count: samples.iter().map(|m| m.error_count).max().unwrap_or(0),
        error_rate: samples.iter().map(|m| m.error_rate).fold(0.0, f64::max),
        unexpected_5xx: samples.iter().map(|m| m.unexpected_5xx).max().unwrap_or(0),
        process_rss_bytes: samples.iter().filter_map(|m| m.process_rss_bytes).max(),
    })
}

fn pct_delta_higher_better(baseline: f64, current: f64) -> Option<f64> {
    if baseline <= 0.0 {
        return None;
    }
    Some((current - baseline) / baseline * 100.0)
}

fn pct_delta_lower_better(baseline: f64, current: f64) -> Option<f64> {
    if baseline <= 0.0 {
        return None;
    }
    Some((current - baseline) / baseline * 100.0)
}

pub fn fingerprints_comparable(
    base: &Fingerprint,
    current: &Fingerprint,
) -> Result<(), Vec<String>> {
    let mut reasons = Vec::new();
    let mut check = |field: &str, a: &str, b: &str| {
        if a != b {
            reasons.push(format!("{field}: {a} != {b}"));
        }
    };
    check("scenario", &base.scenario, &current.scenario);
    check("platform", &base.platform, &current.platform);
    check("arch", &base.arch, &current.arch);
    check("tool.name", &base.tool.name, &current.tool.name);
    check(
        "connections",
        &base.connections.to_string(),
        &current.connections.to_string(),
    );
    check(
        "duration_measure",
        &base.duration_measure,
        &current.duration_measure,
    );
    check("load_mode", &base.load_mode, &current.load_mode);
    check("server_target", &base.server_target, &current.server_target);
    check("tls", &base.tls.to_string(), &current.tls.to_string());
    check(
        "epoll_static",
        &base.epoll_static.to_string(),
        &current.epoll_static.to_string(),
    );
    check("path", &base.path, &current.path);
    check(
        "config_fingerprint",
        &base.config_fingerprint,
        &current.config_fingerprint,
    );
    if reasons.is_empty() {
        Ok(())
    } else {
        Err(reasons)
    }
}

pub fn tolerances_for_gate(gate: Option<&ScenarioGatePolicy>) -> (Tolerances, Option<Tolerances>, Tolerances) {
    match gate {
        None => {
            let d = Tolerances::default();
            (d, Some(d), Tolerances::extreme_default())
        }
        Some(g) => {
            let warn = g
                .warning_threshold
                .as_ref()
                .map(Tolerances::from_bands)
                .unwrap_or_default();
            let fail = if g.hard_gate && !g.failure_threshold_pending {
                Some(
                    g.failure_threshold
                        .as_ref()
                        .map(Tolerances::from_bands)
                        .unwrap_or_default(),
                )
            } else {
                None
            };
            let extreme = g
                .extreme_degradation_threshold
                .as_ref()
                .map(Tolerances::from_bands)
                .unwrap_or_else(Tolerances::extreme_default);
            (warn, fail, extreme)
        }
    }
}

pub fn compare_metrics(
    baseline: &MetricsSnapshot,
    current: &MetricsSnapshot,
    baseline_allowed_errors: f64,
    tolerances: &Tolerances,
) -> Verdict {
    compare_metrics_gated(baseline, current, baseline_allowed_errors, tolerances, Some(tolerances), tolerances)
}

pub fn compare_metrics_gated(
    baseline: &MetricsSnapshot,
    current: &MetricsSnapshot,
    baseline_allowed_errors: f64,
    warn_tol: &Tolerances,
    fail_tol: Option<&Tolerances>,
    extreme_tol: &Tolerances,
) -> Verdict {
    if !baseline.is_complete() || !current.is_complete() {
        return Verdict::InvalidResult(vec!["incomplete metrics".into()]);
    }
    if current.unexpected_5xx > 0 {
        return Verdict::RegressionFail(vec![format!("unexpected_5xx={}", current.unexpected_5xx)]);
    }
    if current.error_rate > baseline_allowed_errors.max(0.0) {
        return Verdict::RegressionFail(vec![format!(
            "error_rate {:.3}% > allowed {:.3}%",
            current.error_rate, baseline_allowed_errors
        )]);
    }

    let mut warnings = Vec::new();
    let mut fails = Vec::new();
    let hard_fail = fail_tol.is_some();

    if let Some(rps_delta) =
        pct_delta_higher_better(baseline.requests_per_second, current.requests_per_second)
    {
        if hard_fail {
            let hard = fail_tol.expect("hard_fail implies Some");
            if rps_delta <= hard.rps_fail_pct {
                fails.push(format!(
                    "RPS {rps_delta:+.1}% (fail <= {}%)",
                    hard.rps_fail_pct
                ));
            } else if rps_delta <= warn_tol.rps_warn_pct {
                warnings.push(format!(
                    "RPS {rps_delta:+.1}% (warn <= {}%)",
                    warn_tol.rps_warn_pct
                ));
            }
        } else if rps_delta <= extreme_tol.rps_fail_pct {
            fails.push(format!(
                "EXTREME RPS {rps_delta:+.1}% (extreme <= {}%)",
                extreme_tol.rps_fail_pct
            ));
        } else if rps_delta <= warn_tol.rps_fail_pct {
            warnings.push(format!(
                "CONDITIONAL: RPS {rps_delta:+.1}% (ordinary regression, FAILURE_THRESHOLD=PENDING_MORE_DATA)"
            ));
        } else if rps_delta <= warn_tol.rps_warn_pct {
            warnings.push(format!(
                "RPS {rps_delta:+.1}% (warn <= {}%)",
                warn_tol.rps_warn_pct
            ));
        }
    } else {
        return Verdict::InvalidResult(vec!["baseline RPS missing".into()]);
    }

    if let Some(p99_delta) = pct_delta_lower_better(baseline.p99_ms, current.p99_ms) {
        if hard_fail {
            let hard = fail_tol.expect("hard_fail implies Some");
            if p99_delta >= hard.p99_fail_pct {
                fails.push(format!(
                    "p99 {p99_delta:+.1}% (fail >= {}%)",
                    hard.p99_fail_pct
                ));
            } else if p99_delta >= warn_tol.p99_warn_pct {
                warnings.push(format!(
                    "p99 {p99_delta:+.1}% (warn >= {}%)",
                    warn_tol.p99_warn_pct
                ));
            }
        } else if p99_delta >= extreme_tol.p99_fail_pct {
            fails.push(format!(
                "EXTREME p99 {p99_delta:+.1}% (extreme >= {}%)",
                extreme_tol.p99_fail_pct
            ));
        } else if p99_delta >= warn_tol.p99_fail_pct {
            warnings.push(format!(
                "CONDITIONAL: p99 {p99_delta:+.1}% (ordinary regression, FAILURE_THRESHOLD=PENDING_MORE_DATA)"
            ));
        } else if p99_delta >= warn_tol.p99_warn_pct {
            warnings.push(format!(
                "p99 {p99_delta:+.1}% (warn >= {}%)",
                warn_tol.p99_warn_pct
            ));
        }
    } else {
        return Verdict::InvalidResult(vec!["baseline p99 missing".into()]);
    }

    if !fails.is_empty() {
        Verdict::RegressionFail(fails)
    } else if !warnings.is_empty() {
        Verdict::Warning(warnings)
    } else {
        Verdict::Pass
    }
}

pub fn compare_run_to_baseline(
    baseline_entry: &BaselineScenarioEntry,
    run: &RunDocument,
    gate: Option<&ScenarioGatePolicy>,
) -> Verdict {
    let (warn_tol, fail_tol, extreme_tol) = tolerances_for_gate(gate);
    if run.schema_version != CONTRACT_SCHEMA_VERSION {
        return Verdict::InvalidResult(vec![format!(
            "run schema_version {} != {}",
            run.schema_version, CONTRACT_SCHEMA_VERSION
        )]);
    }
    if !run.valid {
        return Verdict::InvalidResult(vec![format!(
            "invalid run: {}",
            run.invalid_reason.as_deref().unwrap_or("unknown")
        )]);
    }
    if let Err(reasons) = fingerprints_comparable(&baseline_entry.fingerprint, &run.fingerprint) {
        return Verdict::NotComparable(reasons);
    }
    compare_metrics_gated(
        &baseline_entry.metrics,
        &run.metrics,
        baseline_entry.metrics.error_rate,
        &warn_tol,
        fail_tol.as_ref(),
        &extreme_tol,
    )
}

pub fn compare_median_to_baseline(
    baseline_entry: &BaselineScenarioEntry,
    runs: &[RunDocument],
    gate: Option<&ScenarioGatePolicy>,
) -> Verdict {
    let (warn_tol, fail_tol, extreme_tol) = tolerances_for_gate(gate);
    let valid: Vec<&RunDocument> = runs.iter().filter(|r| r.valid).collect();
    let invalid_count = runs.len().saturating_sub(valid.len());
    if valid.is_empty() {
        return Verdict::InvalidResult(vec!["no valid samples".into()]);
    }
    if invalid_count > 0 {
        // one invalid among three → still compare valid median but note flake
    }
    if let Err(reasons) =
        fingerprints_comparable(&baseline_entry.fingerprint, &valid[0].fingerprint)
    {
        return Verdict::NotComparable(reasons);
    }
    let metrics: Vec<MetricsSnapshot> = valid.iter().map(|r| r.metrics.clone()).collect();
    let Some(median) = median_metrics(&metrics) else {
        return Verdict::InvalidResult(vec!["median unavailable".into()]);
    };
    let mut verdict = compare_metrics_gated(
        &baseline_entry.metrics,
        &median,
        baseline_entry.metrics.error_rate,
        &warn_tol,
        fail_tol.as_ref(),
        &extreme_tol,
    );
    if invalid_count > 0 {
        let note = format!("{invalid_count} invalid sample(s) excluded from median");
        verdict = match verdict {
            Verdict::Pass => Verdict::Warning(vec![note]),
            Verdict::Warning(mut w) => {
                w.push(note);
                Verdict::Warning(w)
            }
            other => other,
        };
    }
    verdict
}

/// Canonical precedence when multiple gate legs fail (highest wins).
/// INVALID_RESULT (2) / cleanup failure > NOT_COMPARABLE (3) > REGRESSION_FAIL (1) > PASS/WARNING (0).
pub fn final_rc_from_exit_codes(codes: &[i32]) -> i32 {
    fn precedence(rc: i32) -> u8 {
        match rc {
            2 => 4,
            3 => 3,
            1 => 2,
            0 => 0,
            _ => 1,
        }
    }
    codes
        .iter()
        .copied()
        .filter(|rc| *rc != 0)
        .max_by_key(|rc| precedence(*rc))
        .unwrap_or(0)
}

pub fn termination_status_from_rc(final_rc: i32) -> &'static str {
    match final_rc {
        0 => "CLEAN_EXIT",
        1 => "REGRESSION_FAIL",
        2 => "INVALID_RESULT",
        3 => "NOT_COMPARABLE",
        _ => "INVALID_RESULT",
    }
}

pub fn merge_verdicts(verdicts: &[Verdict]) -> Verdict {
    if verdicts.is_empty() {
        return Verdict::InvalidResult(vec!["no scenarios to compare".into()]);
    }
    let mut worst = Verdict::Pass;
    for v in verdicts {
        worst = match (&worst, v) {
            (_, Verdict::InvalidResult(_)) => v.clone(),
            (_, Verdict::NotComparable(_)) if !matches!(worst, Verdict::InvalidResult(_)) => {
                v.clone()
            }
            (_, Verdict::RegressionFail(_))
                if !matches!(worst, Verdict::InvalidResult(_) | Verdict::NotComparable(_)) =>
            {
                v.clone()
            }
            (Verdict::Pass, Verdict::Warning(_)) => v.clone(),
            (Verdict::Warning(a), Verdict::Warning(b)) => {
                let mut merged = a.clone();
                merged.extend(b.clone());
                Verdict::Warning(merged)
            }
            _ => worst,
        };
    }
    worst
}

pub fn build_compare_report(
    _baseline: &BaselineDocument,
    lines: Vec<(String, Verdict)>,
) -> CompareReport {
    build_compare_report_with_mode(_baseline, lines, None, None)
}

pub fn overall_verdict_label(merged: &Verdict) -> String {
    match merged {
        Verdict::Pass => "PASS".into(),
        Verdict::Warning(_) => "PASS_WITH_WARNING".into(),
        other => other.as_str().into(),
    }
}

pub fn build_compare_report_with_mode(
    _baseline: &BaselineDocument,
    lines: Vec<(String, Verdict)>,
    mode: Option<&str>,
    collection_status: Option<&str>,
) -> CompareReport {
    let overall = if mode == Some("collect_only") {
        "COLLECT_ONLY".to_string()
    } else {
        let merged = merge_verdicts(&lines.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>());
        overall_verdict_label(&merged)
    };
    CompareReport {
        schema_version: CONTRACT_SCHEMA_VERSION,
        contract_id: CONTRACT_ID.into(),
        overall_verdict: overall,
        mode: mode.map(str::to_string),
        collection_status: collection_status.map(str::to_string),
        scenarios: lines
            .into_iter()
            .map(|(scenario, verdict)| {
                let verdict_label = if mode == Some("collect_only") {
                    "COLLECT_ONLY".to_string()
                } else {
                    verdict.as_str().into()
                };
                let messages = match verdict {
                    Verdict::Warning(m)
                    | Verdict::RegressionFail(m)
                    | Verdict::InvalidResult(m)
                    | Verdict::NotComparable(m) => m,
                    Verdict::Pass => vec![],
                };
                ScenarioCompareLine {
                    scenario,
                    verdict: verdict_label,
                    rps_delta_pct: None,
                    p99_delta_pct: None,
                    messages,
                }
            })
            .collect(),
    }
}

pub fn baseline_from_median_runs(
    doc: &BaselineDocument,
    scenario: &str,
    fingerprint: Fingerprint,
    runs: &[RunDocument],
) -> Option<BaselineScenarioEntry> {
    let metrics: Vec<MetricsSnapshot> = runs
        .iter()
        .filter(|r| r.valid)
        .map(|r| r.metrics.clone())
        .collect();
    let median = median_metrics(&metrics)?;
    Some(BaselineScenarioEntry {
        fingerprint,
        metrics: median,
        repetitions: runs.len() as u32,
        notes: doc.scenarios.get(scenario).and_then(|e| e.notes.clone()),
    })
}

#[cfg(test)]
mod tests {
    use super::super::schema::{ScenarioGatePolicy, ToleranceBands, ToolInfo};
    use super::*;

    fn fp(scenario: &str) -> Fingerprint {
        Fingerprint {
            schema_version: 1,
            scenario: scenario.into(),
            platform: "linux".into(),
            arch: "aarch64".into(),
            kernel: None,
            cpu_count: 4,
            tool: ToolInfo {
                name: "rewrk".into(),
                version: "0.3.2".into(),
            },
            connections: 100,
            threads: 2,
            duration_warmup: "3s".into(),
            duration_measure: "5s".into(),
            load_mode: "ceiling".into(),
            server_target: "exyonq".into(),
            tls: false,
            sendfile: false,
            epoll_static: false,
            path: "/site/1k.bin".into(),
            payload_label: "1KiB".into(),
            features: vec![],
            config_fingerprint: "test".into(),
        }
    }

    fn metrics(rps: f64, p99: f64, error_rate: f64) -> MetricsSnapshot {
        MetricsSnapshot {
            requests_per_second: rps,
            p50_ms: 1.0,
            p95_ms: 2.0,
            p99_ms: p99,
            error_count: if error_rate > 0.0 { 1 } else { 0 },
            error_rate,
            unexpected_5xx: 0,
            process_rss_bytes: None,
        }
    }

    fn baseline_entry(rps: f64, p99: f64) -> BaselineScenarioEntry {
        BaselineScenarioEntry {
            fingerprint: fp("P1"),
            metrics: metrics(rps, p99, 0.0),
            repetitions: 3,
            notes: None,
        }
    }

    #[test]
    fn equal_baseline_passes() {
        let b = baseline_entry(100.0, 10.0);
        let v = compare_metrics(
            &b.metrics,
            &metrics(100.0, 10.0, 0.0),
            0.0,
            &Tolerances::default(),
        );
        assert_eq!(v, Verdict::Pass);
    }

    #[test]
    fn rps_minus_four_passes() {
        let b = baseline_entry(100.0, 10.0);
        let v = compare_metrics(
            &b.metrics,
            &metrics(96.0, 10.0, 0.0),
            0.0,
            &Tolerances::default(),
        );
        assert_eq!(v, Verdict::Pass);
    }

    #[test]
    fn rps_minus_six_warns() {
        let b = baseline_entry(100.0, 10.0);
        let v = compare_metrics(
            &b.metrics,
            &metrics(94.0, 10.0, 0.0),
            0.0,
            &Tolerances::default(),
        );
        assert!(matches!(v, Verdict::Warning(_)));
    }

    #[test]
    fn rps_minus_eleven_fails() {
        let b = baseline_entry(100.0, 10.0);
        let v = compare_metrics(
            &b.metrics,
            &metrics(89.0, 10.0, 0.0),
            0.0,
            &Tolerances::default(),
        );
        assert!(matches!(v, Verdict::RegressionFail(_)));
    }

    #[test]
    fn p99_plus_nine_passes() {
        let b = baseline_entry(100.0, 10.0);
        let v = compare_metrics(
            &b.metrics,
            &metrics(100.0, 10.9, 0.0),
            0.0,
            &Tolerances::default(),
        );
        assert_eq!(v, Verdict::Pass);
    }

    #[test]
    fn p99_plus_twelve_warns() {
        let b = baseline_entry(100.0, 10.0);
        let v = compare_metrics(
            &b.metrics,
            &metrics(100.0, 11.2, 0.0),
            0.0,
            &Tolerances::default(),
        );
        assert!(matches!(v, Verdict::Warning(_)));
    }

    #[test]
    fn p99_plus_twentyfive_fails() {
        let b = baseline_entry(100.0, 10.0);
        let v = compare_metrics(
            &b.metrics,
            &metrics(100.0, 12.5, 0.0),
            0.0,
            &Tolerances::default(),
        );
        assert!(matches!(v, Verdict::RegressionFail(_)));
    }

    #[test]
    fn unexpected_errors_fail() {
        let b = baseline_entry(100.0, 10.0);
        let mut cur = metrics(100.0, 10.0, 0.0);
        cur.unexpected_5xx = 1;
        let v = compare_metrics(&b.metrics, &cur, 0.0, &Tolerances::default());
        assert!(matches!(v, Verdict::RegressionFail(_)));
    }

    #[test]
    fn fingerprint_mismatch_not_comparable() {
        let base_fp = fp("P1");
        let mut cur_fp = fp("P1");
        cur_fp.duration_measure = "30s".into();
        let b = BaselineScenarioEntry {
            fingerprint: base_fp,
            metrics: metrics(100.0, 10.0, 0.0),
            repetitions: 1,
            notes: None,
        };
        let run = RunDocument {
            schema_version: 1,
            contract_id: CONTRACT_ID.into(),
            commit: None,
            captured_at: "0".into(),
            valid: true,
            invalid_reason: None,
            fingerprint: cur_fp,
            metrics: metrics(100.0, 10.0, 0.0),
        };
        let v = compare_run_to_baseline(&b, &run, None);
        assert!(matches!(v, Verdict::NotComparable(_)));
    }

    #[test]
    fn median_of_three_samples() {
        let samples = vec![
            metrics(100.0, 10.0, 0.0),
            metrics(110.0, 11.0, 0.0),
            metrics(120.0, 12.0, 0.0),
        ];
        let m = median_metrics(&samples).expect("median");
        assert_eq!(m.requests_per_second, 110.0);
        assert_eq!(m.p99_ms, 11.0);
    }

    #[test]
    fn invalid_schema_is_invalid() {
        let b = baseline_entry(100.0, 10.0);
        let run = RunDocument {
            schema_version: 99,
            contract_id: CONTRACT_ID.into(),
            commit: None,
            captured_at: "0".into(),
            valid: true,
            invalid_reason: None,
            fingerprint: fp("P1"),
            metrics: metrics(100.0, 10.0, 0.0),
        };
        let v = compare_run_to_baseline(&b, &run, None);
        assert!(matches!(v, Verdict::InvalidResult(_)));
    }

    #[test]
    fn missing_metrics_invalid() {
        let b = baseline_entry(100.0, 10.0);
        let v = compare_metrics(
            &b.metrics,
            &MetricsSnapshot::default(),
            0.0,
            &Tolerances::default(),
        );
        assert!(matches!(v, Verdict::InvalidResult(_)));
    }

    #[test]
    fn overall_pass_with_warning_label() {
        let merged = merge_verdicts(&[
            Verdict::Pass,
            Verdict::Warning(vec!["p99".into()]),
        ]);
        assert_eq!(overall_verdict_label(&merged), "PASS_WITH_WARNING");
    }

    #[test]
    fn conditional_scenario_ordinary_regression_warns_not_fails() {
        let b = baseline_entry(100.0, 10.0);
        let gate = ScenarioGatePolicy {
            reference_quality: "CONDITIONAL".into(),
            hard_gate: false,
            failure_threshold_pending: true,
            minimum_repetitions: 5,
            observed_cv_pct: None,
            warning_threshold: Some(ToleranceBands {
                rps_warn_pct: -5.0,
                rps_fail_pct: -10.0,
                p99_warn_pct: 15.0,
                p99_fail_pct: 25.0,
            }),
            failure_threshold: None,
            extreme_degradation_threshold: Some(ToleranceBands {
                rps_warn_pct: -5.0,
                rps_fail_pct: -25.0,
                p99_warn_pct: 10.0,
                p99_fail_pct: 50.0,
            }),
        };
        let (warn, fail, extreme) = tolerances_for_gate(Some(&gate));
        let v = compare_metrics_gated(
            &b.metrics,
            &metrics(89.0, 10.0, 0.0),
            0.0,
            &warn,
            fail.as_ref(),
            &extreme,
        );
        assert!(matches!(v, Verdict::Warning(_)));
    }

    #[test]
    fn hard_scenario_can_fail() {
        let b = baseline_entry(100.0, 10.0);
        let gate = ScenarioGatePolicy {
            reference_quality: "ACCEPTABLE".into(),
            hard_gate: true,
            failure_threshold_pending: false,
            minimum_repetitions: 5,
            observed_cv_pct: None,
            warning_threshold: None,
            failure_threshold: None,
            extreme_degradation_threshold: None,
        };
        let (warn, fail, extreme) = tolerances_for_gate(Some(&gate));
        let v = compare_metrics_gated(
            &b.metrics,
            &metrics(89.0, 10.0, 0.0),
            0.0,
            &warn,
            fail.as_ref(),
            &extreme,
        );
        assert!(matches!(v, Verdict::RegressionFail(_)));
    }

    #[test]
    fn pending_threshold_extreme_still_fails() {
        let b = baseline_entry(100.0, 10.0);
        let gate = ScenarioGatePolicy {
            reference_quality: "CONDITIONAL".into(),
            hard_gate: false,
            failure_threshold_pending: true,
            minimum_repetitions: 5,
            observed_cv_pct: None,
            warning_threshold: Some(ToleranceBands {
                rps_warn_pct: -5.0,
                rps_fail_pct: -10.0,
                p99_warn_pct: 15.0,
                p99_fail_pct: 25.0,
            }),
            failure_threshold: None,
            extreme_degradation_threshold: Some(ToleranceBands {
                rps_warn_pct: -5.0,
                rps_fail_pct: -25.0,
                p99_warn_pct: 10.0,
                p99_fail_pct: 50.0,
            }),
        };
        let (warn, fail, extreme) = tolerances_for_gate(Some(&gate));
        let v = compare_metrics_gated(
            &b.metrics,
            &metrics(70.0, 10.0, 0.0),
            0.0,
            &warn,
            fail.as_ref(),
            &extreme,
        );
        assert!(matches!(v, Verdict::RegressionFail(_)));
    }

    #[test]
    fn merge_verdicts_empty_is_invalid() {
        let v = merge_verdicts(&[]);
        assert!(matches!(v, Verdict::InvalidResult(_)));
    }

    #[test]
    fn merge_verdicts_not_comparable_over_warning() {
        let v = merge_verdicts(&[
            Verdict::Warning(vec!["p99".into()]),
            Verdict::NotComparable(vec!["duration".into()]),
        ]);
        assert!(matches!(v, Verdict::NotComparable(_)));
    }

    #[test]
    fn merge_verdicts_invalid_over_not_comparable() {
        let v = merge_verdicts(&[
            Verdict::NotComparable(vec!["duration".into()]),
            Verdict::InvalidResult(vec!["missing".into()]),
        ]);
        assert!(matches!(v, Verdict::InvalidResult(_)));
    }

    #[test]
    fn final_rc_invalid_beats_not_comparable() {
        assert_eq!(final_rc_from_exit_codes(&[1, 3, 2]), 2);
    }

    #[test]
    fn final_rc_not_comparable_beats_regression() {
        assert_eq!(final_rc_from_exit_codes(&[1, 3, 0]), 3);
    }

    #[test]
    fn final_rc_all_zero_is_zero() {
        assert_eq!(final_rc_from_exit_codes(&[0, 0, 0]), 0);
    }

    #[test]
    fn final_rc_quick_fail_nonzero() {
        assert_eq!(final_rc_from_exit_codes(&[1, 0, 0]), 1);
    }
}
