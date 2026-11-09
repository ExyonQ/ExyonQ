# Benchmarks documentation (tracked)

```text
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN unless separately authorized
```

## Competitive scoreboard (current cycle)

| Path | Role |
| --- | --- |
| [v044-p1-p14-competitive-dashboard.json](v044-p1-p14-competitive-dashboard.json) | Canonical machine-readable scoreboard |
| [v044-p1-p14-competitive-dashboard.html](v044-p1-p14-competitive-dashboard.html) | Browser dashboard (generated; embeds JSON) |
| Generator | `scripts/benchmarks/generate-competitive-dashboard.py` |

Policy: [performance-competitive-admission.md](../governance/performance-competitive-admission.md)

```text
ENGINEERING_BASELINE ≠ COMPETITIVE_WIN ≠ MIGRATION_VALUE ≠ MARKET_READINESS
```

Regenerate HTML after editing JSON:

```bash
python3 scripts/benchmarks/generate-competitive-dashboard.py
```

## Other tracked HTML

| Path | Note |
| --- | --- |
| [r3-exyonq-vs-ols-fixed-rate-parity.html](r3-exyonq-vs-ols-fixed-rate-parity.html) | Historical R3 fixed-rate proxy parity — not current P1 seal |

## Q-12 reproducible HTTP baselines

- [q12-http-native-2026-11-08.md](q12-http-native-2026-11-08.md) — generated
  native baseline report: medians with 95% bootstrap intervals, RPS, p50/p95/p99
  latency and peak RSS for ExyonQ, nginx and Apache.
- [q12-http-native-2026-11-08.json](q12-http-native-2026-11-08.json) — the same
  report in machine-readable form, consumed by the regression gate.
- Harness: `scripts/bench/run-http-bench.sh` (method and limits in
  [scripts/bench/README.md](../../scripts/bench/README.md)).
- Anti-regression gate: `scripts/gates/bench-regression-gate.sh`.

### Linux CI baseline (2026-11-09)

- [q12-ci-linux-2026-11-09.md](q12-ci-linux-2026-11-09.md) — first native
  Linux CI run (`42695705050`) on GitHub-hosted Ubuntu 24.04, 3 × 5 s,
  16 connections.
- [q12-ci-linux-2026-11-09.json](q12-ci-linux-2026-11-09.json) — exact
  machine-readable report consumed by the gate.
- `measurement_class = ci_native_github_hosted`; this is not a competitive
  claim and the shared-runner noise is part of the result.
- The run was a push on `feat/q12-bench`; `bench-regression` was skipped. The
  `main` baseline and a passing PR gate remain pending because `main` is 0.9.1.

### Main Linux CI baseline (2026-11-09)

- [q12-main-linux-2026-11-09.md](q12-main-linux-2026-11-09.md) — real
  `main` baseline from run `42705724344` at revision `68d1257`; this is the
  artifact consumed by the PR regression gate.
- [q12-main-linux-2026-11-09.json](q12-main-linux-2026-11-09.json) — exact
  machine-readable report.

### Native OLS Linux CI baseline (2026-11-09)

- [q12-ols-linux-2026-11-09.md](q12-ols-linux-2026-11-09.md) — first native
  OpenLiteSpeed run (`42699088690`) on GitHub-hosted Ubuntu 24.04, with
  OpenLiteSpeed 1.9.4-3+noble and the same 3 × 5 s / 16-connection method.
- [q12-ols-linux-2026-11-09.json](q12-ols-linux-2026-11-09.json) — exact
  machine-readable report, including all four servers and both payloads.
- All 24 raw repetitions report `successRate = 1.0`, only HTTP 200 responses
  and zero errors; the run log ends with `RESULT=PASS Q12_HTTP_BENCH`.
- This run is separate from `42695705050`; the host CPU differs, so its
  numbers are not comparable across runs. It is not a `main` baseline.

`measurement_class = local_native_non_canonical` applies to the macOS arm64
baseline; it is not canonical Linux evidence and not a competitive claim.
OpenLiteSpeed is **NOT EXECUTED** on macOS, but it **was executed natively in
Linux CI** (`42699088690`, OpenLiteSpeed 1.9.4-3+noble). The branch CI baseline
and the `main` baseline are executed; the PR gate uses host-normalized ratios
so cross-runner shifts do not masquerade as regressions.
