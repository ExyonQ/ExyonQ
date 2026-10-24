# V044 — PXDP root cause review (governance)

```text
WIP = V044_PXDP_ROOT_CAUSE_AND_REMAINING_P4_DEFICIT_REVIEW
STATUS = CLOSED
CASE = RC-B
PXDP_PERFORMANCE_REALITY_CHECK = ACCEPTED_CLOSED (PR-C)
OWNER_DECISION = FREEZE_BROAD_PXDP_FEATURE_EXPANSION
PRODUCT_MUTATION = NO
REQUEST_BODY_RELAY = NOT_AUTHORIZED
PXDP_P6_IMPLEMENTATION_AUTHORIZED = NO
```

Performance report: [`../performance/v044-pxdp-root-cause-p4-deficit-review.md`](../performance/v044-pxdp-root-cause-p4-deficit-review.md)

## Authority chain

| Step | Verdict |
|------|---------|
| P5 admission (P5-A) | CLOSED_QUALIFIED |
| Performance reality check | **PR-C** — PXDP did not materially fix P4 deficit |
| Root cause review | **RC-B** — multiple material roots proven |

## Terminal metrics (profiling run `20260827-013039`)

```text
CURRENT_EXYONQ_USER_CPU_US_PER_REQ = 19.92
CURRENT_EXYONQ_SYSTEM_CPU_US_PER_REQ = 16.76
CTX_SWITCHES_PER_REQ = 127.90
FUTEX_SYSCALLS_PER_REQ = 8.41
EPOLL_SYSCALLS_PER_REQ = 0.53
WRITE_SYSCALLS_PER_REQ = 0.011
CLIENT_ACCEPTS_PER_REQ = 0.003

PROVEN_ROOT_CAUSES =
  SCHEDULER_CTX_SWITCH_EXCESS,
  TOKIO_FUTEX_AND_EPOLL_REACTOR,
  KERNEL_CROSS_CORE_IPI,
  HYPER_UPSTREAM_CLIENT_INTEGRATION_RESIDUAL,
  BYTES_REFCOUNT_CHURN,
  USER_CPU_SAVED_NOT_CONVERTED_TO_RPS

NEXT_REPAIR_PROPOSAL =
  OWNER_REVIEW targeted Tokio/Hyper upstream + scheduler geometry reduction
  (no request-body relay; no P6; no broad feature expansion)
```

## Owner freeze (unchanged)

```text
PXDP_MATERIAL_PERFORMANCE_SUCCESS = NO
PXDP_ARCHITECTURE_VALUE = PARTIAL
PXDP_PERFORMANCE_HYPOTHESIS = WEAK_OR_FAILED
P4_PERFORMANCE_MIGRATION_BARRIER = REMAINS
CANONICAL_MERGE = NO
PUSH = NO
TAG = NO
RELEASE = NO
```

Evidence: `.exyonq-local/evidence/pxdp-root-cause-p4-deficit/20260827-013039/`
