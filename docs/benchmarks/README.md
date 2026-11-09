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

## Q-12 reproducible HTTP baseline (2026-11-08)

- [q12-http-native-2026-11-08.md](q12-http-native-2026-11-08.md) — generated
  native baseline report: medians with 95% bootstrap intervals, RPS, p50/p95/p99
  latency and peak RSS for ExyonQ, nginx and Apache.
- [q12-http-native-2026-11-08.json](q12-http-native-2026-11-08.json) — the same
  report in machine-readable form, consumed by the regression gate.
- Harness: `scripts/bench/run-http-bench.sh` (method and limits in
  [scripts/bench/README.md](../../scripts/bench/README.md)).
- Anti-regression gate: `scripts/gates/bench-regression-gate.sh`.

`measurement_class = local_native_non_canonical`: real executions on a declared
macOS arm64 host, not canonical Linux evidence, and not a competitive claim.
OpenLiteSpeed is **NOT EXECUTED** on that host; the harness now accepts a
native OLS process on Linux via a loopback URL/PID. CI execution is
**NOT EXECUTED**: `.github/workflows/bench.yml` and the baseline gate exist,
but there is no push or first successful `main` run yet.
