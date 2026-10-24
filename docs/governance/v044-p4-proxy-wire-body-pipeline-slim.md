# V044 — P4 proxy wire body pipeline slim (HOLD)

```text
STATUS = HOLD
WIP = V044_P4_PROXY_WIRE_BODY_PIPELINE_SLIM
MODE = OWNER_AUTHORIZED_ISOLATED_PRODUCT_EXPERIMENT_AB (SUSPENDED)
SUSPENDED_BY = V044_DATA_PLANE_ARCHITECTURE_STRATEGIC_REVIEW
SUPERSEDED_BY = V044_PROXY_EXECUTION_DATA_PLANE_ARCHITECTURE_DESIGN (PXDP-B)
SUSPENDED_DATE = 2026-08-26

PRODUCT_MUTATION = NO (canonical tree)
EXPERIMENT_BRANCH = experiment/p4-proxy-wire-body-pipeline-slim (worktree only; not authority)
EXPERIMENT_WORKTREE = /Volumes/Lexar/Cursor/exyonq-laboratorio/.exyonq-local/worktrees/p4-body-pipeline-slim
EXPERIMENT_COMMIT = a7dac179 (local experiment only; not merged)
CANONICAL_PRODUCT_CHANGED = NO
NON_PRODUCTION_EXPERIMENT = YES

SHOULD_PROCEED = NO
SHOULD_RESUME = NO
  # owner may authorize diagnostic-only A/B separately; not recommended before PXDP-P2+
  # See v044-data-plane-architecture-strategic-review.md CASE DP-C
```

## Intent (when authorized)

Isolated A/B for **ROOT_1A_3 + ROOT_1G_2**: slim `hyper::body::Incoming` → wire writer
without `BoxBody` erasure on P4 proxy hot path. Same Cap034 A0 framing (chunked + close).

## Hold reason

Strategic review concluded **profile-by-profile slice optimization** on P4 is not sustainable
while proxy execution architecture remains fragmented. Body-pipeline slim may be diagnostically
useful but must not proceed as product path until **V044_PROXY_EXECUTION_DATA_PLANE_ARCHITECTURE_DESIGN**
(owner decision).

## State at hold

| Item | Status |
|------|--------|
| Experiment code | EXISTS in isolated worktree only |
| Netcup A/B | NOT_COMPLETED (build aborted; strategic hold) |
| Terminal CASE BP-A..E | NOT_EMITTED |
| Canonical merge | FORBIDDEN |

```text
NEXT_ACTION = NONE until owner releases hold or authorizes diagnostic-only A/B
```
