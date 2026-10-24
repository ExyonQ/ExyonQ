# Benchmarks documentation (tracked)

```text
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN unless separately authorized
```

## Competitive scoreboard (current cycle)

| Path | Role |
|------|------|
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
|------|------|
| [r3-exyonq-vs-ols-fixed-rate-parity.html](r3-exyonq-vs-ols-fixed-rate-parity.html) | Historical R3 fixed-rate proxy parity — not current P1 seal |
