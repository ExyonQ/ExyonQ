# Cap067 / proxy-wire cheap modules (ratelimit + metrics)

```text
STATUS = IMPLEMENTED_AE_GATE_COMPRESSION_COEXIST
WIP = CAP067_WIRE_CHEAP_MODULES
PRODUCT_MUTATION = YES
MEASUREMENT_CLASS = DEVELOPMENT
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
CONTRACT = CONDITIONAL_ACCEPT early-AE-gate-only (no mid-conn Cap067→Hyper)
```

## Problem

With `[modules.ratelimit]` + `[modules.metrics]` enabled, `modules_enabled()` forced
**all** TCP onto Hyper CrossCuttingPipeline. Cap067 EPOLL remasure stayed ~45–50k
while OLS ~120k on P4/P10/P12 (DEV). Later, `hyper_required_for_modules() =
filter_chain.compression` still forced Hyper whenever compression was configured,
even for AE-idle wrk (no Accept-Encoding).

## ADR — Cap067 wire-cheap × compression coexistence

**Decision (early gate only):**

| Predicate | Behavior |
|-----------|----------|
| `compression_configured()` | IR `filter_chain.compression` |
| `hyper_required_for_modules()` | always false for compression (no headerless force) |
| `modules_require_hyper_for_request_head(head)` | configured ∧ AE prefers gzip/br/deflate/zstd (q>0) |
| `wire_cheap_modules_active()` | ratelimit and/or metrics (independent of compression) |
| Drain / WS upgrade / OpenMetrics path | Hyper as today |

**Keepalive:** Cap067 ownership is sticky for the TCP connection. A later request
with `Accept-Encoding: gzip` on a Cap067-owned socket does **not** mid-handoff to
Hyper (`P4-LA-001`); Cap067 serves identity (compression filter never runs on wire).
Clients that need compression must send AE on the first request (Hyper from admit).

**Non-goals this tranche:** Cap067 RESPONSE_MUTATING compression; WASM; second reactor.

## Wire-cheap path

- **admit** → Cap055 `PROCESS_MODULE_LIMITER` (peer IP)
- **record_response** → Cap054 process-lifetime counters
- OpenMetrics scrape (`GET /metrics`) stays Hyper (LA-CAP054-008)
- Dynamic compression body filters stay Hyper-only after early AE gate

## Causal (Netcup DEV 20260923)

| Config | P4 `/api/` med RPS |
|--------|--------------------:|
| modules OFF | ~122k |
| modules ON + compression=true (old all-TCP Hyper) | ~45–56k |
| modules ON + compression=false (wire-cheap) | **~126k** |
| modules ON + compression=true + AE-idle (AE gate) | **~136k** |
| modules ON + compression=true + AE:gzip (Hyper) | ~56k |

AE-gate causal (`p10p12-ae-gate-20260923T145238Z`, DEV≠official):

| corridor | exyonq med | ols med |
|----------|----------:|--------:|
| AE-idle (compression ON) | ~136k | ~123k |
| AE:gzip | ~56k | — |

Final wire-cheap matrix (`p10p12-wire-cheap-final-20260923T135804Z`, DEV≠official):

| sc | exyonq | ols | vs ols |
|----|-------:|----:|-------:|
| P4 | 126115 | 116511 | +8.2% |
| P10 | 122542 | 117681 | +4.1% |
| P12 | 115353 | 114570 | +0.7% |

PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN (DEV≠official).

## Remasure gate

P4/P10/P12 with `bench-modules.toml` **compression enabled** + Cap067 EPOLL ON +
wrk AE-idle (no Accept-Encoding) must stay wire-cheap band (~120k+). Separate
AE:gzip corridor measured as Hyper-pipeline (DEV≠official).

Post P5-handoff + metrics-scrape pipeline fix (`p10p12-modules-epoll-on-20260923T153150Z`, DEV≠official):

| sc | exyonq med | ols med | vs ols |
|----|----------:|--------:|-------:|
| P4 | 135540 | 127292 | +6.5% |
| P10 | 132845 | 126634 | +4.9% |
| P12 | 130674 | 123223 | +6.0% |

`/metrics` OpenMetrics 200 under AE-idle + compression ON (LA-CAP054-008 pipeline path).
nginx numbers in that suite are contaminated (container paused) — ignore.
Nginx remasure after upstream keepalive fix (`…/keepalive-fix`, DEV≠official): P4 med ~104k, 0 errors (was EADDRNOTAVAIL/502 without upstream keepalive).
