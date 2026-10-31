# PXDP-P5 Architecture — Policy & Feature Admission

**WIP:** `V044_PROXY_EXECUTION_DATA_PLANE_POLICY_AND_FEATURE_ADMISSION`  
**Parent:** P4-A framing + keepalive lifecycle

## Purpose

P5 is **admission-first**: inventory → classify → freeze ledger → bounded implementation of ADMIT_NOW / ADMIT_AFTER_BOUNDED_REPAIR only.

PXDP does not inherit legacy reachability by default:

```text
FEATURE_EXISTS_IN_LEGACY != SAFE_FOR_PXDP
```

## Dispatch mutual exclusion

```text
hyper_required_for_modules()           → false for compression (headerless)
modules_require_hyper_for_request_head → compression configured ∧ AE prefers coding
  → Hyper + CrossCuttingPipeline (body filters)
AE-idle + ratelimit/metrics            → Cap067 / proxy-wire + sync hooks
modules empty                          → Cap067 / proxy-wire eligible
```

`modules_enabled()` is true when `RuntimeSnapshot.module_state` is non-empty (`runtime_plan.rs`).
It no longer forces Hyper by itself. Compression forces Hyper only via the Cap067
**early Accept-Encoding gate** (first-request headers). No mid-connection Cap067→Hyper
handoff (`P4-LA-001`). OpenMetrics scrape path stays Hyper (`LA-CAP054-008`).

**Consequence:** Cap055 ratelimit admit + Cap054 metrics record are reachable on the PXDP /
proxy-wire scope via `wire_module_hooks` even when compression is configured (AE-idle).
Full Hyper Service body filters run only when AE prefers a content coding. WAF
(including abuse rate-limit facet via `evaluate_wire_waf`) **is** on the wire path.

## P5 bounded tranche (implementation)

| Item | Type | Change |
| --- | --- | --- |
| POL-05 generation on keepalive | repair | `read_state(shared)` each P4 loop iteration |
| QUAL-KEEPALIVE-RELOAD-G1G2 | qualification | same-socket R1@G1 → reload → R2@G2 |
| QUAL-KEEPALIVE-100-SEQ | qualification | 100 sequential GET on one connection |
| QUAL-BODYLESS-204 | qualification | upstream 204 + keepalive R1/R2 |

No new HTTP methods. No request-body relay. No policy engine rewrite.

## Admission boundary (conceptual)

```text
AdmitPxdp           → P4 loop executes
RejectCanonical       → WAF/route/cache deny before PXDP
CloseUnsupported      → unsupported next request on owned keepalive socket
DeferToLegacyBeforeOwnership → first request only, before P4 takes socket (P4 minimizes this)
```

Same-connection PXDP→Hyper handoff remains **forbidden** (P4-LA-001 class).

## P4 deferred items — P5 disposition

| Item | P5 result |
| --- | --- |
| KEEPALIVE_RELOAD_G1_G2 | **PASS** (same-socket qual + generation refresh) |
| KEEPALIVE_100_REQUEST_SEQUENCE | **PASS** (qual harness) |
| KEEPALIVE_IDLE_TIMEOUT | **EXPLICIT_NONBLOCKING** (no new idle timer in P5; documented) |

## Migration coverage matrix (conceptual — NOT migration ready)

| Workload facet | PXDP | Legacy only | Deferred |
| --- | --- | --- | --- |
| GET /api reverse proxy (no modules) | SUPPORTED_IN_PXDP | — | — |
| Config rate limit on proxy | — | LEGACY_ONLY | wire projection WIP |
| POST/PUT bodies | — | LEGACY_ONLY | REQUEST_BODY WIP |
| WebSocket / CONNECT | — | LEGACY_ONLY | UPGRADE_TUNNEL WIP |
| H2 / H3 | — | LEGACY_ONLY | dedicated execution WIP |
| Module filters | — | LEGACY_ONLY | module parity WIP |

## Hot-path invariants preserved

```text
NEW_EXYONQ_TASKS_PER_REQUEST = 0
NEW_EXYONQ_CHANNELS_PER_REQUEST = 0
NEW_GLOBAL_HOT_PATH_LOCKS = 0 (read_state uses existing shared RwLock — not new)
POLICY_DUPLICATION_ADDED = NO
P3_BACKPRESSURE = preserved (MAX_IN_FLIGHT body frames bounded)
```

## Evidence

Darwin: `cargo test -p exyonq-core --lib pxdp_p5 -- --test-threads=1`  
Netcup: evidence driver retired with the v0.4.4 campaign (script no longer in tree)  
Ledger SSOT: `docs/governance/v044-proxy-execution-data-plane-feature-admission-ledger.md`

## Terminal performance validation (PXDP reality check)

P5 admission closure does **not** prove P4 performance. Measured validation:

- Performance: [`../performance/v044-pxdp-performance-reality-check.md`](../performance/v044-pxdp-performance-reality-check.md)
- Governance: [`../governance/v044-pxdp-performance-reality-check.md`](../governance/v044-pxdp-performance-reality-check.md)
- Verdict: **PR-C** — PXDP did **not** materially close the P4 proxy execution deficit; request-body relay **not** authorized.
- Follow-on: [`../performance/v044-pxdp-root-cause-p4-deficit-review.md`](../performance/v044-pxdp-root-cause-p4-deficit-review.md) (**RC-B**).
