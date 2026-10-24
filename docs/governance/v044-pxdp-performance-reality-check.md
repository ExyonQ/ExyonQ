# V044 — PXDP performance reality check (governance)

```text
WIP = V044_PXDP_PERFORMANCE_REALITY_CHECK
STATUS = ACCEPTED_CLOSED
CASE = PR-C
PXDP_P5_STATUS = CLOSED_QUALIFIED (P5-A) — prerequisite only
PRODUCT_MUTATION = NO
REQUEST_BODY_RELAY = NOT_AUTHORIZED
PXDP_P6_IMPLEMENTATION_AUTHORIZED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
```

Performance report: [`../performance/v044-pxdp-performance-reality-check.md`](../performance/v044-pxdp-performance-reality-check.md)

## Authority

| Item | Value |
|------|-------|
| Run ID | `20260826-223445` |
| Evidence | `.exyonq-local/evidence/pxdp-performance-reality/20260826-223445/` |
| P5 terminal HEAD | `0bc2b973e5ff62b2316fbc7c3f002d673c65d791` |
| P5 terminal TREE | `b1b7659b4d3bb8745ea0ed42a8de7d424f523767` |
| Old P4 authority | `v044-p4-proxy-1k-3way-20260822T115113Z` |
| Excluded invalid run | `v044-p4-proxy-1k-3way-20260822T112732Z` |

## Integrity

| Gate | Status |
|------|--------|
| BENCHMARK_INTEGRITY | PASS (no product semantic shortcuts; WAF off; no modules) |
| DATA_PROVENANCE | PASS (real execution; HAProxy history not fabricated) |
| CORRECTNESS | PASS (200 + 1024 B + body hash parity all products) |
| CURRENT_COMPETITOR_COMPARABILITY | PASS_WITH_NGINX_HARNESS_FIX |

## Terminal verdict

```text
V044_PXDP_PERFORMANCE_REALITY_CHECK_STATUS = CLOSED
CASE = PR-C
ARCHITECTURE_HYPOTHESIS = WEAK_OR_FAILED
DID_PXDP_MATERIALLY_FIX_THE_P4_PROXY_EXECUTION_DEFICIT = NO

PXDP_RPS_GAIN_VS_OLD_PERCENT = 4.17
PXDP_CPU_PER_REQ_IMPROVEMENT_VS_OLD_PERCENT = 2.59
PXDP_USER_CPU_IMPROVEMENT_VS_OLD_PERCENT = 14.87

EXYONQ_VS_NGINX_RPS = -20.60
EXYONQ_VS_OLS_RPS = -24.01
EXYONQ_VS_HAPROXY_RPS = +122.59  # current measurement only; HAProxy not historical peer

SHOULD_WE_INVEST_NEXT_IN_REQUEST_BODY_RELAY = NO
P4_PERFORMANCE_MIGRATION_BARRIER = REMAINS
GLOBAL_MIGRATION_VALUE = NOT_PROVEN
MARKET_READINESS = NOT_READY
```

## Owner actions implied

1. **STOP** broad PXDP feature expansion (including request-body relay and P6) pending root-cause / architecture review.
2. Do **not** treat P5 correctness/admission closure as performance success.
3. User-CPU improved ~15% but **RPS gap vs NGINX/OLS unchanged** — hypothesis not materially validated on primary gates.
4. Promote canonical `nginx.conf` keepalive block into any future PXDP benchmark worktrees before competitive runs.

## Five states (separate)

| State | Verdict |
|-------|---------|
| Architecture correctness (P5) | CLOSED_QUALIFIED |
| PXDP performance value (P4) | **NOT demonstrated** |
| P4 competitive position | **BEHIND** |
| Global migration value | NOT_PROVEN |
| Market readiness | NOT_READY |

## Next WIP proposal (owner-gated)

`V044_PXDP_ROOT_CAUSE_AND_REMAINING_P4_DEFICIT_REVIEW` — read-only profiling + stage/task accounting vs pre-PXDP baseline; **no** request-body relay until owner re-admits.
