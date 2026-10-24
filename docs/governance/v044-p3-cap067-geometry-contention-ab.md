# V044 P3 Cap067 — geometry contention A/B (8 vs 4)

```text
STATUS = COMPLETE
WIP = V044_P3_CAP067_GEOMETRY_CONTENTION_AB
PRODUCT_MUTATION = NO
VERDICT = FALSIFIED_REVERT_KEEP_GEOM8
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

SUITE = v044-p1p3-geometry-contention-ab-20260917T144507Z
HOST = netcup-bench
IMAGE = v044p1auth-exyonq-chunkab
BINARY_SHA256 = fefe547d13a056de36fc43a5fab1feac8bc7ab957e3a65db318689806aa49d7c
KEEP_CLAMP = EXYONQ_SENDFILE_CHUNK=131072
EVIDENCE_NETCUP =
  /root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p1p3-geometry-contention-ab-20260917T144507Z/
LAB_COPY =
  .exyonq-local/evidence/v044-p3-cap067-geometry-contention-ab-20260917T144507Z/
HARNESS = scripts/remote/v044-p3-cap067-geometry-contention-ab-remote.sh
PRIOR_CAUSAL = docs/governance/v044-p3-cap067-large-body-causal.md
```

## Hypothesis

Causal profile: P3 residual **SYSTEM_DOMINATED** with spinlock/memcg pressure under
parallel large-body send. Reducing Cap067 parallelism (accept/worker/epoll **8→4**)
might cut lock/memcg contention and improve P3 RPS / core-ms.

## Arms (same binary + clamp)

| Arm | Geometry |
|-----|----------|
| A | ACCEPT/WORKER/EPOLL_POOL = **8** |
| B | ACCEPT/WORKER/EPOLL_POOL = **4** |

## Results (ExyonQ medians)

| Arm | P1 RPS | P1 p99 | P2 RPS | P2 p99 | P3 RPS | P3 p99 | P3 core-ms |
|-----|--------|--------|--------|--------|--------|--------|------------|
| A geom8 | 149230 | 1.44 | 89995 | 2.70 | **6109** | 32.46 | **0.412** |
| B geom4 | 156298 | 1.34 | 82270 | 2.58 | 6035 | 33.81 | 0.430 |

### Δ B vs A

| Profile | dRPS% | dP99% | dCMS% |
|---------|-------|-------|-------|
| P1 | **+4.7%** | −6.9% | −10.2% |
| P2 | **−8.6%** | −4.4% | −4.1% |
| P3 | **−1.2%** | +4.2% | **+4.4%** |

### Protectors (vs A)

Gate: P1/P2 RPS drop ≤ 3%; p99 worsen ≤ 10%.

```text
PROTECTOR_P1 = PASS
PROTECTOR_P2 = FAIL   # RPS −8.6%
```

## Verdict

```text
GEOMETRY_CONTENTION_HYPOTHESIS = FALSIFIED
KEEP_GEOM4_AS_DEFAULT = NO
KEEP_GEOM8 = YES
RATIONALE =
  P3 does not improve (RPS −1.2%, core-ms +4.4%).
  P2 protector FAIL. Do not ship geom4 as Cap067 default.
```

P1 alone liking geom4 is **not** sufficient — static DoD requires P2 protectors and P3 lift.

## Next (after falsification)

From causal + productivity diag:

1. **MEMCG packaging** — **DEFERRED_INFEASIBLE** (root memory controller on Netcup)  
2. **Sendfile productivity** — COMPLETE: ExyonQ 8×128 KiB vs NGINX 1×1 MiB  
3. **Large-body cork OFF A/B** — admitted next (`EXYONQ_LARGE_BODY_TCP_CORK=0`)  
4. **Sendfile vs nginx splice** — deferred until cork-off A/B completes  

Do **not** reintroduce invalidated socket knobs.

## Complexity

```text
Complexity Added:   harness + this note (env-only A/B)
Complexity Removed: 0
Net Complexity:     +small (diagnostic; no product KEEP)
```
