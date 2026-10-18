# ADR-054 — Runtime `EXY-RT-*` diagnostics (Q-02 / v0.5 F1)

**Status:** Accepted
**Date:** 2026-10-18
**Depends on:** ADR-051 (maps/headers), Cap021 (connect retry), Cap040 (drain probes)
**Contract:** CONDITIONAL ACCEPT (`exyonq-architecture-contract` — checklist below)

## Context

QUEJAS_NGINX §2.2–2.3: operators get opaque 502/503/504 and silent “variable didn’t match” outcomes. Q-01 covers offline predictive lint; Q-02 covers **serve-path** failure diagnostics with stable `EXY-RT-*` codes plus route/upstream context.

## Decision

### Emission rules

| Rule | Requirement |
|------|-------------|
| Failure-only | Emit on final failed outcome (after Cap021 retries exhausted), drain reject, or unknown `$map` reference — **never** on successful 2xx/3xx proxy completions |
| Status unchanged | Diagnostics are logs only; HTTP status mapping stays Cap021/Cap040 |
| Context | Prefer `cluster_id`, `path`, `peer` (when known), `host`, `cause` |
| Layer | `exyonq-mod-proxy` (upstream) + `core` (drain) + `exyonq-config-ir` notes for map expand; codes registered in `DiagnosticCode` |

### Codes

| Code | When | Typical status |
|------|------|----------------|
| `EXY-RT-0001` | Final upstream connect failure (includes DNS resolve failure) | 502 |
| `EXY-RT-0002` | Final upstream attempt / shared-deadline timeout | 504 |
| `EXY-RT-0003` | No eligible peer / missing cluster binding | 503 |
| `EXY-RT-0004` | Final unretryable upstream transport/protocol error | 502 |
| `EXY-RT-0005` | `$map` name referenced in `proxy_set_headers` but no `[[map]]` | (header expand; request may still proceed) |
| `EXY-RT-0006` | New non-probe work rejected because process is draining | 503 |

`cause` field values (when set): `dns` | `connect` | `timeout` | `no_peer` | `upstream` | `map_unknown` | `draining`.

### Map / “regex sin match”

Regex map keys remain **fail-closed at migrate** (`F1-R16` / ADR-051). Exact-key miss that applies a configured `default` is **intentional** and must not spam logs on the success path. Q-02 surfaces only **unknown map names** (`EXY-RT-0005`).

### Non-goals

- Changing client-visible response bodies to embed `EXY-RT-*`
- Periodic upstream DNS re-resolve (QUEJAS P2; separate track)
- Success-path map/default tracing
- New IR fields or hot-path allocations beyond structured `tracing` fields

## Contract checklist (CONDITIONAL ACCEPT)

1. ADR-054 exists and codes are in `DiagnosticCode` + `docs/config/diagnostic-codes.md`
2. Proxy final 502/503/504 paths emit matching `EXY-RT-0001`…`0004`
3. Drain reject emits `EXY-RT-0006`
4. Unknown `$map` emits `EXY-RT-0005` without success-path exact-key spam
5. Unit + reality E2E prove codes appear in logs; status codes unchanged
6. F1_RESIDUALS `F1-W02` closed; Roadmap Q-02 checked

## Consequences

- Operator logs on proxy failures name cause + cluster/path/peer.
- Q-02 closes F1-W02; transversal (`F1-W04`) remains for matrix ticks.
