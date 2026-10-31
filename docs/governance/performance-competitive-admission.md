# Performance competitive admission (ExyonQ)

```text
RULE_ID = PERFORMANCE-COMPETITIVE-ADMISSION
STATUS = ACTIVE
SCOPE = P1–P14 + scale variants V1/V10/V100/V1000 where supported

PRODUCT_UNDER_TEST = EXYONQ

PRINCIPAL_RIVALS = NGINX | OPENLITESPEED | HAPROXY
PRINCIPAL_RIVAL_COUNT = 3
TOTAL_PRODUCTS_IN_COMPETITIVE_MATRIX = 4

COMPARATORS = NGINX | OpenLiteSpeed | HAProxy (when equivalent comparison is supported)

FEATURE_EQUIVALENCE_REQUIRED = YES
ASYMMETRIC_FEATURE_COST_MUST_BE_DISCLOSED = YES
CLAIM_SCOPE <= EVIDENCE_SCOPE

HISTORICAL_P1_P4_HAPROXY = NOT_MEASURED
RETROACTIVE_SYNTHETIC_HAPROXY_VALUES = FORBIDDEN
FAIL_MODE = FAIL_CLOSED_ON_OVERCLAIM
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN unless separately authorized
```

## Forbidden implication

```text
ENGINEERING_BASELINE = CLOSED
  ≠ COMPETITIVE_WIN
  ≠ MIGRATION_VALUE
  ≠ MARKET_READINESS
```

`CLOSED_CURRENT_COMPETITIVE_BASELINE` is an **engineering** seal. It does **not**
prove a commercial win or a reason to migrate from NGINX, OpenLiteSpeed, or HAProxy.

## Independent state axes

Every profile/scale combination maintains independent fields:

| Axis | Values |
| --- | --- |
| `TECHNICAL_CORRECTNESS` | PASS / FAIL / NOT_VERIFIED |
| `ENGINEERING_BASELINE` | CLOSED / OPEN / NOT_MEASURED |
| `PERFORMANCE_COMPETITIVENESS` | AHEAD / PARITY / BEHIND / MIXED / NOT_MEASURED |
| `COMPETITIVE_WIN` | YES / NO / NOT_PROVEN |
| `MIGRATION_VALUE` | PROVEN / PARTIAL / NOT_PROVEN |
| `MARKET_READINESS` | READY / NOT_READY / NOT_ASSESSED |

Never derive one automatically from another.

## Scoreboard source of truth

```text
CANONICAL_DATA = docs/benchmarks/v044-p1-p14-competitive-dashboard.json
DASHBOARD_HTML = docs/benchmarks/v044-p1-p14-competitive-dashboard.html
GENERATOR = scripts/benchmarks/generate-competitive-dashboard.py
```

HTML is generated from JSON. Do not hand-edit numeric truth in HTML.

Missing values use: `NOT_MEASURED` | `NOT_AVAILABLE` | `HISTORICAL_ONLY` — never guessed.

## Core metrics (minimum)

Throughput (RPS) · CPU total / per-request / user / system · memory RSS / peak ·
latency P50 / P95 / P99 · errors / reliability · stability (CV or equivalent).

Per metric: `WINNER_*` ∈ {EXYONQ, NGINX, OLS, HAPROXY, TIE, NOT_MEASURED}.

Historical P1–P4 HAProxy cells must be `NOT_MEASURED` — never empty, zero, inferred, or estimated.

## HAProxy equivalence (future benchmarks)

When HAProxy is included in a competitive run, the benchmark contract must define
equivalent semantics for (where relevant):

```text
protocol · keepalive · upstream reuse · connection-close behavior · TLS · HTTP version
logging · security features · observability · compression · timeouts · connection limits
worker/core geometry
```

TCP-mode vs HTTP-mode HAProxy is **not** interchangeable unless the contract explicitly
names the mode. Asymmetric feature cost must be disclosed in the governance doc for that run.

## Pareto product rule (`COMPETITIVE_WIN = YES`)

Not “win every cell.” Require:

1. Material wins in multiple important dimensions; **and**
2. No material disadvantage in remaining critical dimensions; **or**
3. A **verified** functional/operational advantage that compensates a measured
   performance disadvantage.

Roadmap / future features are **not** compensation.

“Similar” or “not much worse” ⇒ `MIGRATION_VALUE = NOT_PROVEN`.

## Migration value gate

For each profile/scale answer:

```text
WHY_MIGRATE_FROM_NGINX =
WHY_MIGRATE_FROM_OLS =
WHY_MIGRATE_FROM_HAPROXY =
MIGRATION_VALUE_VS_NGINX = PROVEN | PARTIAL | NOT_PROVEN
MIGRATION_VALUE_VS_OLS = PROVEN | PARTIAL | NOT_PROVEN
MIGRATION_VALUE_VS_HAPROXY = PROVEN | PARTIAL | NOT_PROVEN | NOT_MEASURED
```

## No code without expected product value

```text
NO_CODE_WITHOUT_EXPECTED_PRODUCT_VALUE = YES
```

Before any performance product optimization:

```text
EXPECTED_USER_VALUE =
EXPECTED_METRIC_GAIN =
MINIMUM_ACCEPTABLE_GAIN =
RISK_BUDGET =
ROLLBACK_CRITERIA =
CAUSAL_EVIDENCE_STRENGTH = PROVEN | STRONGLY_SUPPORTED | BOUNDED | HYPOTHESIS
```

Optimization from `HYPOTHESIS` alone is forbidden.

Normally require at least one materially significant expected gain in RPS, CPU,
memory, P95, or P99. Tiny expected gains need ≈zero complexity and ≈zero risk.

## Anti–IA-churn ledger

Every future optimization records:

```text
OPTIMIZATION_ID =
PROMISED_GAIN =
ACTUAL_GAIN =
PRODUCT_DEFECTS_INTRODUCED =
REPAIR_WIPS_REQUIRED =
AUDIT_WIPS_REQUIRED =
NET_ENGINEERING_VALUE =
```

If defects + complexity outweigh final gain ⇒ REVERT / REJECT. Sunk cost ≠ value.

## P2 / later profiles

```text
P2_EXECUTION = COMPLETE (CASE P2-C2)
P2_STARTED = YES (measurement done; P3=NO)
DOC = docs/governance/v044-p2-static-64k-competitive-baseline.md
```

until owner reauthorizes under this governance. Historical numbers may appear
as `HISTORICAL_ONLY` / `SUPERSEDED`, never as current authority.

## Related

- Engineering P1 seal: `docs/governance/v044-p1-final-post-repair-baseline.md`
- Cap067 FD reuse: `docs/governance/v044-p1-fd-reuse-repair.md`
- Cap067 miss hang: `docs/governance/v044-p1-cap067-miss-needhyper-hang.md`

## P1 competitive deficit attribution (2026-08-21)

```text
WIP = V044_P1_COMPETITIVE_DEFICIT_ATTRIBUTION
CASE = C2
STATUS = COMPLETE_NO_JUSTIFIED_PRODUCT_REPAIR
RECOMMENDED_ROOT = NONE
NEXT_PRODUCT_WIP_PROPOSAL = NONE
DOC = docs/governance/v044-p1-competitive-deficit-attribution.md
```

Engineering seal remains CLOSED. Competitive win / migration value remain NOT_PROVEN.

## P3 static 1 MiB (2026-08-21)

```text
P3_EXECUTION = COMPLETE (CASE P3-C1)
P3_COMPETITIVE_WIN = YES
P3_MIGRATION_VALUE = PARTIAL (vs NGINX and vs OLS)
P3_MARKET_READINESS = NOT_READY
P4_STARTED = NO
DOC = docs/governance/v044-p3-static-1mib-competitive-baseline.md
```

## Architecture fitness before P4 (2026-08-22)

```text
WIP = V044_ARCHITECTURE_FITNESS_REVIEW_BEFORE_P4
STATUS = COMPLETE
PRODUCT_MUTATION = NO
ARCHITECTURE_FITNESS = TARGETED_ARCHITECTURAL_EVOLUTION
MAJOR_REARCHITECTURE_JUSTIFIED = NO
IO_URING_RECOMMENDATION = NOT_JUSTIFIED_YET
P4_STARTED = NO
DOC = docs/governance/v044-architecture-fitness-review-before-p4.md
```

## P1 fixed-stage & parallelism A/B (2026-08-22)

```text
WIP = V044_P1_FIXED_STAGE_AND_PARALLELISM_A_B
STATUS = COMPLETE
CASE = AB3
PRODUCT_MUTATION = NO
RECOMMENDED_ROOT = PARALLELISM_UNDERUTIL
A1_RPS_DELTA_PCT ≈ +10.4 (WT/AW/EPOLL_POOL 4→8 matched)
HANDLE_REGISTRY_ROOT = REJECTED_NOT_MATERIAL
NEXT_PRODUCT_WIP_PROPOSAL = SUPERSEDED_BY_GEOMETRY8
P4_STARTED = NO
DOC = docs/governance/v044-p1-fixed-stage-and-parallelism-ab.md
```

Historical worker A/B without `EXYONQ_EPOLL_POOL_THREADS` is not authority.

## P1 default worker geometry (2026-08-22)

```text
WIP = V044_P1_DEFAULT_WORKER_GEOMETRY_8
STATUS = CLOSED_VERIFIED_REAL_PRODUCTION
CASE = G1
PRODUCT_MUTATION = YES  # coordinated available_parallelism defaults
PRODUCT_COMMIT = ade997a362e60d1dc6eafc984da454930600845a
PARALLELISM_UNDERUTIL = CLOSED_FIXED_AND_REBENCHMARKED
P1_COMPETITIVE_WIN = NOT_PROVEN
P2_COMPETITIVE_WIN = NOT_PROVEN
P3_COMPETITIVE_WIN_AFTER = YES
P4_STARTED = NO
DOC = docs/governance/v044-p1-default-worker-geometry-8.md
SCOREBOARD = docs/benchmarks/v044-p1-p14-competitive-dashboard.json
```

## P4 proxy 1 KiB (2026-08-22)

```text
P4_EXECUTION = COMPLETE (CASE P4-C2)
P4_VALIDITY = PASS
P4_COMPARABILITY = PASS
P4_TECHNICAL_CORRECTNESS = PASS
P4_ENGINEERING_BASELINE = CLOSED
P4_COMPETITIVE_WIN = NOT_PROVEN
P4_MIGRATION_VALUE_VS_NGINX = NOT_PROVEN
P4_MIGRATION_VALUE_VS_OLS = NOT_PROVEN
P4_MARKET_READINESS = NOT_READY
DOES_THIS_RESULT_INCREASE_MIGRATION_VALUE = NO
PRODUCT_MUTATION = NO
HARNESS_ONLY_FIX = YES  # nginx upstream keepalive + P4 harness smoke gate
INVALID_RUN = v044-p4-proxy-1k-3way-20260822T112732Z (INVALID_NON_AUTHORITATIVE)
VALID_RUN = v044-p4-proxy-1k-3way-20260822T115113Z
P5_STARTED = NO
DOC = docs/governance/v044-p4-proxy-1k-competitive-baseline.md
SCOREBOARD = docs/benchmarks/v044-p1-p14-competitive-dashboard.json
```

## Cross-profile root cause review P1–P4 (2026-08-25)

```text
WIP = V044_CROSS_PROFILE_PERFORMANCE_ROOT_CAUSE_REVIEW_P1_P4
STATUS = COMPLETE
CASE = B   # MULTIPLE_BOUNDED_ROOTS_IDENTIFIED
PRODUCT_MUTATION = NO
P5_STARTED = NO
P5_TO_P14_FROZEN = YES
ARCHITECTURE_FITNESS_AFTER_P4 = TARGETED_ARCHITECTURAL_EVOLUTION
MAJOR_REARCHITECTURE_JUSTIFIED = NO
FULL_REWRITE_JUSTIFIED = NO
GLOBAL_MIGRATION_VALUE = NOT_PROVEN
MARKET_READINESS = NOT_READY
DOC = docs/governance/v044-cross-profile-performance-root-cause-review-p1-p4.md
NEXT_PRODUCT_WIP_PROPOSAL = NONE
NEXT_MEASUREMENT_WIP_PROPOSAL = V044_P4_PROXY_PATH_CAUSAL_PROFILING
```

## HAProxy comparator reconciliation (2026-08-25)

```text
HAPROXY_GOVERNANCE_RECONCILIATION = COMPLETE
DOCS_COMPARATOR_SET = NGINX / OPENLITESPEED / HAPROXY
DASHBOARD_SUPPORTS_HAPROXY = YES
P1_HAPROXY = NOT_MEASURED
P2_HAPROXY = NOT_MEASURED
P3_HAPROXY = NOT_MEASURED
P4_HAPROXY = NOT_MEASURED
HISTORICAL_BENCHMARKS_ARE_THREE_WAY = YES
```

## P4 proxy path causal profiling (2026-08-25)

```text
WIP = V044_P4_PROXY_PATH_CAUSAL_PROFILING
STATUS = COMPLETE
CASE = P4P-B
PRODUCT_MUTATION = NO
HAPROXY_GOVERNANCE_RECONCILIATION = COMPLETE
P5_STARTED = NO
P5_TO_P14_FROZEN = YES
DOC = docs/governance/v044-p4-proxy-path-causal-profiling.md
EVIDENCE = v044-p4-proxy-path-causal-profiling-20260825T203611Z
NEXT_PRODUCT_WIP_PROPOSAL = NONE
NEXT_MEASUREMENT_WIP_PROPOSAL = NONE
```

## Cap034 framing / keep-alive bounded-value A/B (2026-08-25)

```text
WIP = V044_CAP034_FRAMING_KEEPALIVE_BOUNDED_VALUE_AB
STATUS = COMPLETE
CASE = AB-C34-B
NON_PRODUCTION_EXPERIMENT = YES
DOC = docs/governance/v044-cap034-framing-keepalive-bounded-value-ab.md
EVIDENCE = v044-cap034-framing-keepalive-ab-20260825T222608Z (Netcup amd64)
A1_VS_A0_RPS_DELTA_PERCENT = +5.27
DOES_CAP034_EVOLUTION_MATERIALLY_CLOSE_P4_GAP = NO
EXPECTED_MIGRATION_VALUE_EFFECT = PARTIAL
NEXT_PRODUCT_WIP_PROPOSAL = NONE
```

## Cap034 proxy framing & keep-alive contract review (2026-08-25)

```text
WIP = V044_CAP034_PROXY_FRAMING_AND_KEEPALIVE_CONTRACT_REVIEW
STATUS = COMPLETE
CASE = C34-C
PRODUCT_MUTATION = NO
DOC = docs/governance/v044-cap034-proxy-framing-keepalive-contract-review.md
IS_CAP034_REQUIRED_BY_HTTP = NO
IS_CAP034_REQUIRED_BY_CURRENT_WIRE_IMPLEMENTATION = YES
SAFE_CONTRACT_EVOLUTION = OPTION_C (general downstream framing policy) — review only
EXPECTED_USER_VALUE = NOT_BOUNDED
NEXT_PRODUCT_WIP_PROPOSAL = NONE
NEXT_MEASUREMENT_WIP_PROPOSAL = NONE
```

## Cap034 framing / keep-alive bounded-value A/B (2026-08-25)

```text
WIP = V044_CAP034_FRAMING_KEEPALIVE_BOUNDED_VALUE_AB
STATUS = COMPLETE
CASE = AB-C34-B
NON_PRODUCTION_EXPERIMENT = YES
DOC = docs/governance/v044-cap034-framing-keepalive-bounded-value-ab.md
EVIDENCE = v044-cap034-framing-keepalive-ab-20260825T222608Z (Netcup amd64)
A1_VS_A0_RPS_DELTA_PERCENT = +5.27
DOES_CAP034_EVOLUTION_MATERIALLY_CLOSE_P4_GAP = NO
EXPECTED_MIGRATION_VALUE_EFFECT = PARTIAL
NEXT_PRODUCT_WIP_PROPOSAL = NONE
  # superseded for primary P4 root by V044_P4_PROXY_WIRE_BODY_PIPELINE_SLIM (PX-B)
```

## P4 proxy execution tax decomposition (2026-08-25)

```text
WIP = V044_P4_PROXY_EXECUTION_TAX_DECOMPOSITION
STATUS = COMPLETE
CASE = PX-B
PRODUCT_MUTATION = NO
DOC = docs/governance/v044-p4-proxy-execution-tax-decomposition.md
EVIDENCE = v044-p4-proxy-execution-tax-decomposition-20260825T230314Z (Netcup amd64)
P4P_B_PROFILE_REPRODUCED = YES
ROOT_PRIORITY_1 = ROOT_1A_3 + ROOT_1G_2 (Hyper body/frame adapter slimming)
TOTAL_NON_OVERLAPPING_EXPECTED_RPS_GAIN_RANGE = BOUNDED 12–22%
NEXT_PRODUCT_WIP_PROPOSAL = V044_PROXY_EXECUTION_DATA_PLANE_ARCHITECTURE_DESIGN
  # superseded 2026-08-26: strategic review CASE DP-C; body-pipeline slim HOLD
NEXT_MEASUREMENT_WIP_PROPOSAL = NONE
CAP034_PRODUCTIZATION_PRIORITY = DEFER
```

## Data plane architecture strategic review (2026-08-26)

```text
WIP = V044_DATA_PLANE_ARCHITECTURE_STRATEGIC_REVIEW
STATUS = COMPLETE
CASE = DP-C
DOC = docs/governance/v044-data-plane-architecture-strategic-review.md
RECOMMENDED_ARCHITECTURE_DIRECTION = dedicated proxy execution data plane (design only)
V044_P4_PROXY_WIRE_BODY_PIPELINE_SLIM = HOLD
NEXT_DESIGN_WIP_PROPOSAL = NONE  # completed → PXDP-B
P5_STARTED = NO
```

## Proxy execution data plane architecture design (2026-08-26)

```text
WIP = V044_PROXY_EXECUTION_DATA_PLANE_ARCHITECTURE_DESIGN
STATUS = COMPLETE
CASE = PXDP-B
PRODUCT_MUTATION = NO
DOC = docs/governance/v044-proxy-execution-data-plane-architecture-design.md
CANONICAL_DESIGN = docs/architecture/v044-proxy-execution-data-plane-architecture-design.md
RECOMMENDED_DESIGN = worker-local ProxyConnection shell; Hyper protocol engine only
EXPECTED_RPS_GAIN = PLAUSIBLE_NOT_PROVEN
NEXT_PRODUCT_WIP_PROPOSAL = NONE  # owner must authorize PXDP-P0 implementation
V044_P4_PROXY_WIRE_BODY_PIPELINE_SLIM = HOLD
SHOULD_BODY_PIPELINE_RESUME = NO
```
