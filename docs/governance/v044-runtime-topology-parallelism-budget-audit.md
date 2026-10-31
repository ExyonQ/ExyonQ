# V044 — Runtime topology & parallelism budget (governance)

```text
WIP = V044_RUNTIME_TOPOLOGY_AND_SECOND_TOKIO_POOL_ATTRIBUTION
CASE = RT-C
STATUS = CLOSED_READ_ONLY
PRODUCT_MUTATION = NO
CANONICAL_DEFAULT_CHANGE = NO
MULTI_CONCURRENCY_SWEEP = HOLD
PUSH = NO
TAG = NO
RELEASE = NO
```

Architecture: [`../architecture/v044-runtime-topology-parallelism-budget-audit.md`](../architecture/v044-runtime-topology-parallelism-budget-audit.md)  
Performance: [`../performance/v044-runtime-topology-parallelism-budget-audit.md`](../performance/v044-runtime-topology-parallelism-budget-audit.md)  
Parent: [`v044-exyonq-tokio-worker-geometry-ab.md`](v044-exyonq-tokio-worker-geometry-ab.md)  
Related: [`v044-p1-default-worker-geometry-8.md`](v044-p1-default-worker-geometry-8.md) (G1 SoT)

## Verdict

| Field | Value |
| --- | --- |
| Tokio runtimes in serve | **1** |
| Block B | Cap067 OS pool (not 2nd Tokio RT) |
| Fan-out | Independent CPU-derived pools — **proven** |
| Global budget | **NO** |
| Defect (duplicate RT) | **NOT_PROVEN** |
| Next product repair | **NONE** |

## Authority

```text
PUBLICATION = NO
CANONICAL_MERGE = NO
WORKER_DEFAULT_CHANGE = NOT_AUTHORIZED
```

## Follow-on (closed)

Cap067 process-residency P4 cost A/B → **CP-C** / `CLOSED_NOT_MATERIAL`.  
SoT: [`v044-p4-cap067-process-residency-ab.md`](v044-p4-cap067-process-residency-ab.md) · [`../performance/v044-p4-cap067-process-residency-ab.md`](../performance/v044-p4-cap067-process-residency-ab.md)

Global execution architecture strategic review → **GA-B** (hybrid; Tokio keep / Cap067 preserve / proxy specialize).  
SoT: [`v044-global-execution-architecture-strategic-review.md`](v044-global-execution-architecture-strategic-review.md)

```text
P4_MICRO_OPTIMIZATION_CAMPAIGN = STOPPED
PXDP_P6 = HOLD
REQUEST_BODY_RELAY = HOLD
MULTI_CONCURRENCY_SWEEP = HOLD
```
