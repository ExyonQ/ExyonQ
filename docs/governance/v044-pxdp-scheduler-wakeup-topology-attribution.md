# V044 — PXDP scheduler wakeup topology attribution (governance)

```text
WIP = V044_PXDP_SCHEDULER_WAKEUP_TOPOLOGY_ATTRIBUTION
STATUS = CLOSED
CASE = SW-C
MODE = READ_ONLY_SCHEDULER_CAUSAL_ATTRIBUTION
PRODUCT_MUTATION = NO
HARNESS_MUTATION = NO
CANONICAL_MERGE = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
```

Performance narrative: [`../performance/v044-pxdp-scheduler-wakeup-topology-attribution.md`](../performance/v044-pxdp-scheduler-wakeup-topology-attribution.md)

## Authorization chain

| Prior WIP | Outcome |
|-----------|---------|
| RC-B | Multiple material classes; ctx ~128/req |
| UG-C | **Owner accepted closed** — SendRequest-owner path **rejected** for current P4 profile |
| **This WIP** | Read-only deeper edge attribution; **no product repair** |

## Invalid evidence (mandatory)

| Artifact | Classification |
|----------|----------------|
| `a1-20260827-022600` ctx/req ~7.9 | **INVALID_SCOPE_MISMATCH** — never cite |

Authoritative UG-C fair A/B: ctx **127.9 → 135.7**, RPS **−5%**.

## Terminal cases

| Case | When |
|------|------|
| SW-A | One dominant wakeup edge proven |
| SW-B | Small set of dominant edges proven |
| **SW-C** | **Cross-core / migration / involuntary preemption dominant** ← **this run** |
| SW-D | Connect/close lifecycle dominant |
| SW-E | Runtime topology dominant, no single edge |
| SW-F | No actionable root |
| SW-G | Insufficient evidence |

## Prohibitions honored

- No product / architecture / harness mutation
- No SendRequest-owner continuation
- No worker-count / affinity / thread-per-core tuning in primary run
- No Hyper/Tokio replacement, Bytes/allocator work, P6, request-body relay
- No repair recommendation without **≥7% plausible RPS** causal basis → **NONE**

## Next step

```text
NEXT_PRODUCT_REPAIR = NONE
RETURN_TO_OWNER_REVIEW = YES
```

Do not stack micro-optimizations without new owner authorization.
