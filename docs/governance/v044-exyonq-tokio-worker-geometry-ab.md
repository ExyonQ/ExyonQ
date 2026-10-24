# V044 — ExyonQ Tokio worker geometry A/B (governance)

```text
WIP = V044_EXYONQ_TOKIO_WORKER_GEOMETRY_AB
MODE = READ_ONLY_CONFIGURATION_AB_AND_CAUSAL_VALIDATION
PRODUCT_MUTATION = NO
PARENT = V044_MINIMAL_RUST_HYPER_TOKIO_PROXY_CONTROL (MC-B)
CANONICAL_DEFAULT_CHANGED = NO
PUBLICATION = NO
```

## Authority

Bounded configuration A/B on ExyonQ PXDP P5 terminal binary — **no source changes**, **no default change**.

Parent MC-B showed minimal Hyper/Tokio control worker-count response; this WIP measures **ExyonQ itself**.

## Worker control (source truth)

From `core/src/server/runtime_parallelism.rs`:

| Knob | Override | Default when unset |
|------|----------|-------------------|
| Tokio workers | `EXYONQ_WORKER_THREADS` | `available_parallelism().clamp(1,16)` |
| Accept workers | `EXYONQ_ACCEPT_WORKERS` | same default |
| Cap067 epoll pool | `EXYONQ_EPOLL_POOL_THREADS` | follows accept |

**This WIP varied `EXYONQ_WORKER_THREADS` only** — accept/pool remain at detected parallelism (8 on cpuset). **Observed `tokio-rt-worker` thread count can exceed the env value** (e.g. requested 8 → 16 threads measured). Isolation is **PARTIAL**.

**RT-C clarification:** the “extra” named `tokio-rt-worker` cohort is **Cap067 OS keepalive threads** inheriting the parent Tokio worker name — not a second Tokio runtime. See [`v044-runtime-topology-parallelism-budget-audit.md`](v044-runtime-topology-parallelism-budget-audit.md).

## Decision cases

| Case | Meaning |
|------|---------|
| WG-A | ≥7% RPS vs fresh w16 + ≥20% ctx reduction + correctness |
| WG-B | 3–7% RPS or strong scheduler move without full economic gate |
| WG-C | Not material |
| WG-D | Throughput gain with regression elsewhere |
| WG-E | Invalid / non-isolatable |

## Related

- Performance: [`../performance/v044-exyonq-tokio-worker-geometry-ab.md`](../performance/v044-exyonq-tokio-worker-geometry-ab.md)
- Parent: [`../performance/v044-minimal-rust-hyper-tokio-proxy-control.md`](../performance/v044-minimal-rust-hyper-tokio-proxy-control.md)
