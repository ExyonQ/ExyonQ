# ExyonQ Benchmark Integrity Policy

```text
POLICY_ID = EXYONQ-BENCHMARK-INTEGRITY-ZERO-SHORTCUTS
POLICY_STATUS = MANDATORY
CURSOR_RULE = .cursor/rules/exyonq-benchmark-integrity-zero-shortcuts.mdc
GATE = scripts/gates/benchmark-integrity-gate.sh
FAIL_MODE = FAIL_CLOSED
```

## Principle

```text
BENCHMARK_CODE_MUST_NOT_CHANGE_PRODUCT_SEMANTICS
NORMAL_EXECUTION_BEHAVIOR = BENCHMARK_EXECUTION_BEHAVIOR
```

```text
Nunca optimices ExyonQ para reconocer o favorecer un benchmark.

Optimiza el producto real.

Un benchmark solo es válido cuando mide el mismo camino, trabajo y semántica
que una solicitud normal equivalente.

Cualquier atajo específico de benchmark, incluso temporal o bien intencionado,
es una violación crítica de integridad y debe bloquear el benchmark y la release.
```

## Motivating incident

```text
BENCH_API_CACHE_PATHS = ["/api/", "/api/health"]
```

In `exyonq-mod-proxy`, without `[[cache_policy]]`, the first request hit upstream and later requests reused a stored body. The benchmark stopped measuring a real reverse proxy.

Classification: **CRITICAL** product semantic shortcut — regardless of intent, duration, or “bench-only” framing.

## Allowed vs forbidden variance

**Allowed:** telemetry, observational instrumentation, metrics capture, logging, profiling, external orchestration, fixtures, resource limits, load generation.

**Forbidden to diverge for benchmarks:** routing, response bodies/headers (semantic), cache, proxy semantics, protocol behavior, body generation, compression, connection handling, retries, security/authz, error handling, upstream access, filesystem access, network path.

## Absolute prohibitions

Product code must not introduce semantic branches keyed on:

```text
BENCH_* | BENCHMARK_* | PERF_TEST_* | FAST_BENCH_* | LOAD_TEST_*
SPECIAL_CASE_FOR_BENCHMARK
```

Also forbidden: hard-coded benchmark paths/bodies; automatic cache for bench paths; precomputed bodies outside the normal flow; bypass of proxy/FS/protocol/auth; loadgen or User-Agent detection; fixture/hostname/scenario special cases; error reclassification to pass a run; score gaming; excluding unfavorable results; response reuse without public policy; bench features in normal release builds; temporary bench code on the productive hot path.

## Cache rule

```text
NO EXPLICIT CACHE POLICY → NO RESPONSE CACHE
```

Only public, documented configuration (e.g. `[[cache_policy]]` or the canonical equivalent) may enable caching, and must honor full cache contract (key, query, Host, Authorization, Cookie, Vary, TTL, invalidation, cacheability headers, tenants, method, status, content-encoding).

Forbidden: internal allowlists such as `BENCH_API_CACHE_PATHS`. Forbidden: path-only cache unless a public policy explicitly and safely requests it.

## Legitimate optimizations

Accept only when the change: improves normal product use; preserves observable semantics; does not depend on bench paths/loadgens; has functional tests; is documented as a general optimization; can ship in production; does not falsify work performed; does not skip required upstream/handler/protocol; passes bench vs non-bench comparison.

## Feature flags

Instrumentation features: default OFF; no response/routing/timing/cache/protocol change; observational only. Names containing `bench|benchmark|perf|profiling|diagnostic|instrumentation` must be classified `OBSERVATIONAL_ONLY` or `SEMANTIC_CHANGE`. Semantic → merge/release/benchmark blocked.

## Agent declaration (before product edits)

All of the following must be `NO`:

```text
DOES_CHANGE_PRODUCT_SEMANTICS
DOES_CHANGE_BENCHMARKED_WORK
DOES_ADD_BENCHMARK_SPECIAL_CASE
DOES_ADD_PATH_SPECIAL_CASE
DOES_ADD_LOADGEN_DETECTION
DOES_ADD_IMPLICIT_CACHE
DOES_SKIP_REQUIRED_COMPONENT
DOES_CHANGE_ERROR_CLASSIFICATION
```

Otherwise: `IMPLEMENTATION_AUTHORIZED = NO`.

## Reverse-proxy causality

Without explicit cache:

```text
N_CLIENT_REQUESTS = N_UPSTREAM_REQUESTS
```

(with documented retries: `N_UPSTREAM_REQUESTS >= N_CLIENT_REQUESTS`). Never fewer upstream hits than client requests. When upstream stops: `STALE_200_WITH_PREVIOUS_BODY = FORBIDDEN` unless an explicit stale-cache contract is under test.

## Static / protocol

Benchmarks must not insert bodies into handlers, skip FS when FS is in contract, use compiled fixtures when measuring real reads, or special-case measured paths. Protocol errors, partial bodies, and resets must not be scored as clean success.

## Process gates

```text
pre-benchmark qualification
pre-competitive-run
pre-release
local debugger
architecture review
security review
```

```text
BENCHMARK_INTEGRITY_GATE != PASS → competitive run FORBIDDEN
PRODUCT_SEMANTIC_BENCHMARK_SHORTCUTS > 0 → release close FORBIDDEN
UNKNOWN_BENCHMARK_SHORTCUTS > 0 → release close FORBIDDEN
```

## Concealment ban

Agents must not reclassify defects as mere methodology without evidence, swap scenarios to hide defects, weaken contracts without authorize, present unequal work as equivalent, destroy evidence, or continue phases with an open critical integrity violation.

On detection: stop; preserve evidence; root-cause; notify owner; await authorize; fix product; re-run correctness; re-run only the affected benchmark.

## Review checklist (perf changes)

1. Benefits a normal request?  
2. Same handler?  
3. Same upstream/FS accesses?  
4. Same bytes?  
5. Same errors/protocol semantics?  
6. Any benchmark-related condition?  
7. Valid with random path?  
8. Valid with random body?  
9. Valid with another loadgen?  
10. Valid outside the harness?  

Any `NO`/`UNKNOWN` → `BENCHMARK_RESULT_VALID = NO`, `MERGE_ALLOWED = NO`.
