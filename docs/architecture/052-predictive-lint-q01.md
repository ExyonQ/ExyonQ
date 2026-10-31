# ADR-052 — Predictive config lint (Q-01 / v0.5 F1)

**Status:** Accepted  
**Date:** 2026-10-18  
**Depends on:** ADR-047 (match ranking), Cap024 (health_check), ADR-051 (headers/maps)  
**Contract:** ACCEPT (`exyonq-architecture-contract` — Q-01 before AP-07)

## Context

QUEJAS_NGINX §2.1–2.3 ask for operator warnings on shaded routes, duplicate headers, upstreams without health checks, and surprising regex. IR validation is fail-closed for structural errors; footguns that still parse must not be silent.

## Decision

### Layering

| Pass | Owner | Severity | Default exit |
| --- | --- | --- | --- |
| Parse + IR validate | `exyonq-config-ir` / merge | `Error` | fail |
| Predictive lint | `exyonq-config-cli` after successful load | `Warning` | 0 unless `--strict` |

Predictive analysis is **offline**, **pure on IR** (no DNS, no probes, no `core/`), and runs only when `exyonqctl config lint --warn` (or `check --warn`) is set.

### Codes (`EXY-LINT-xxxx`)

| Code | Rule |
| --- | --- |
| `EXY-LINT-0001` | Prefix/suffix route partially shaded by a longer same-host pattern |
| `EXY-LINT-0002` | Duplicate `proxy_set_headers` name on one route |
| `EXY-LINT-0003` | Upstream with endpoints and `health_check.enabled = false` |
| `EXY-LINT-0004` | Same-host regex with shorter `pattern_len` than another regex (loses when both match) |
| `EXY-LINT-0005` | `[[map]]` never referenced from any `proxy_set_headers` value |
| `EXY-LINT-0006` | Hostless catch-all `/` while host-specific routes exist (host rank wins) |
| `EXY-LINT-0007` | FPC serve window (`max_ttl_seconds + stale_if_error_secs`) exceeds the anonymous-nonce tick (43200s) |

Every warning carries `cause` + `suggestion`. Documentation pointers use `docs/config/diagnostic-codes.md#exy-lint-000N`.

### Non-goals (this slice)

- AP-07 Include expansion  
- Promoting advisories to IR `Error`  
- Runtime / serve-path diagnostics (Q-02)  
- Live health probes or DNS at lint time  

## Consequences

- Migrate matrices that call `lint` without `--warn` stay `OK` on clean IR.  
- `--strict --warn` fails the process when advisories fire.  
- Residual / follow-up: richer dominance (header AND, rewrite rematch) stays out of Q-01.
