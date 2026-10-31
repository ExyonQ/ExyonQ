# V044 P1 — CPU efficiency live three-way (ExyonQ / NGINX / OLS)

```text
STATUS = COMPLETE
ROOT3_PRODUCT_COMMIT = 2462410ec93fc54d50e6ef1f977d71e17f370450
EXYONQ_BINARY_SHA256 = f243ef2c07a41c88c14a86c1241598051a24d0eeac891f96c319b9ab5ce49f1d
EVIDENCE = .exyonq-local-evidence/v044-p1-cpu-live-3way-20260821T032643Z/
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
P2_STARTED = NO
```

## Contract

Static 1 KiB · HTTP/1.1 keepalive · 100 connections · 2 rewrk threads · 20s warmup · 30s measure · 5 reps · cpuset 0–7 · CpuQuota=0 · same Docker model.

OLS URL authority: `http://openlitespeed-latest:8088/site/1k.bin` (listens on **8088**).

## Calculation method

```text
RPS = rewrk --json requests_avg; median of 5 reps
CPU_avg = mean of docker stats CPUPerc samples in each measure window; median across reps
req/core = RPS / (CPU_avg%/100)
core-ms/req = 1000 / req_per_core   # CPU_COST_PER_SUCCESSFUL_REQUEST
P99 = rewrk --pct after the 5-rep series
```

## Live results (median)

| | ExyonQ (Root1+Root3) | NGINX stable | OLS latest |
| --- | --- | --- | --- |
| RPS | 150317.86 | 158753.80 | 132838.55 |
| RPS CV | 0.084 | 0.039 | 0.124 |
| CPU avg % | 199.86 | 217.48 | 137.38 |
| req/core | 75210.99 | 72998.10 | 96694.24 |
| **core-ms/req** | **0.013296** | **0.013699** | **0.010342** |
| P50 / P90 / P95 / P99 (ms) | 0.96 / 1.29 / 1.39 / **1.62** | 1.08 / 1.42 / 1.52 / **1.79** | 1.03 / 1.37 / 1.47 / **1.66** |

```text
EXYONQ_CPU_COST_VS_NGINX_PERCENT = -2.94%   # ExyonQ cheaper than NGINX on core-ms/req
EXYONQ_CPU_COST_VS_OLS_PERCENT = +28.56%    # ExyonQ more expensive than OLS
```

## Decision

```text
CASE = B  # live three-way decision (vs NGINX closed / vs OLS material)
P1_CPU_EFFICIENCY_VS_NGINX = CLOSED_COMPETITIVE
P1_CPU_EFFICIENCY_VS_OLS = MATERIAL_GAP
OVERALL_CPU_DEFECT_STATUS = OPEN_PROVEN_OLS_DIFFERENTIAL__FD_GEOMETRY_MATERIAL
  # FD cost attribution COMPLETE — see v044-p1-ols-fd-geometry-cost-attribution.md (CASE A)
```

## Read-only FD discrimination (strace -c, ~15s load)

| Syscall/req (approx) | ExyonQ | OLS |
| --- | --- | --- |
| openat2 / openat | **≈1.0** | ≈0.0001 |
| statx / fstat | **≈1.0** | ≈0.0001 |
| close | **≈1.0** | ≈0.0002 |
| sendfile | ≈1.0 | 0 |
| writev | 0 | ≈1.0 |

```text
OLS_FD_REUSE_DIFFERENTIAL = PROVEN_MATERIAL
FD_GEOMETRY_EXPLAINS_PERCENT = 54-58
  # of clean cgroup TOTAL µs/req gap (BOUNDED); see v044-p1-ols-fd-geometry-cost-attribution.md
  # docker-stats +28.56% remains live status authority; absolute µs meters differ
REMAINING_UNEXPLAINED_CPU_GAP = USERSPACE_~4.35_US_PER_REQ_PLUS_NON_FD_KERNEL
FD_REUSE_PRODUCT_REPAIR = NOT_AUTHORIZED
COST_ATTRIBUTION = docs/governance/v044-p1-ols-fd-geometry-cost-attribution.md
```

## Design proposal only (NOT implemented)

```text
DESIGN_PROBLEM =
  Cap067 P1 currently pays openat2+statx+close per successful static request while
  OLS keeps a hot static FD open (writev path), correlating with lower core-ms/req.

EXISTING_PATTERNS =
  Cap004 openat2(RESOLVE_BENEATH); Cap067 sendfile handle registry (Hyper path);
  generation-scoped StaticRuntime / reload publish; Root1 worker LocalRuntimePin.

LIFETIME_INVARIANTS =
  FD must not outlive generation/root/asset validity; no use-after-close; no leak;
  no symlink escape; no path/inode confusion after reload.

GENERATION_INVALIDATION_MODEL =
  Any live FD cache must invalidate on generation bump, static root rebind, and
  asset identity change (inode/mtime/size or equivalent).

CACHE_KEY =
  generation + resolved root id + relative path identity (and/or inode+dev) —
  exact key TBD before any implementation.

STALE_FD_FAILURE_MODES =
  serve wrong generation content; serve deleted/replaced inode; Cap004 bypass;
  FD leak under reload; TOCTOU between validate and sendfile.

PATTERN_CONSIDERED =
  [generation-scoped FD cache, worker-local FD cache, immutable asset descriptor,
   direct open per request, NONE]

NO_NEW_PATTERN_ALTERNATIVE =
  Keep direct openat2 per request (current Cap004-safe path) until owner authorizes
  a generation-scoped design that preserves RESOLVE_BENEATH.

PATTERN_SELECTED = NONE
WHY_SELECTED = Repair not authorized; diagnostic only.
WHY_ALTERNATIVES_REJECTED =
  Implementing FD reuse now would violate FD_REUSE_PRODUCT_REPAIR=NOT_AUTHORIZED
  and requires a full Cap004/reload contract review before mutation.
```

Harness: `scripts/remote/v044-p1-cpu-live-3way.sh`

## Superseding current authority (post Cap067 FD-reuse + H1)

This document remains the **historical** Root1+Root3 live-3way record (−2.94% / +28.56%).

Current P1 seal after product commits through Cap067 miss/NeedHyper hang repair:

```text
SEE = docs/governance/v044-p1-final-post-repair-baseline.md
PRODUCT_AUTHORITY_HEAD = 06742e1bdc67b5ed86ef95750399e9edad16d646
P1_FINAL_STATUS = CLOSED_CURRENT_COMPETITIVE_BASELINE
CASE = S1
  # docker-stats: vs NGINX −15.89% core-ms; vs OLS +9.58% (soft; not a new causal WIP)
DO_NOT_USE_THIS_FILE_AS_CURRENT_OLS_GAP = YES
```

```text
STOP FOR OWNER REVIEW
P2_STARTED = NO
PUSH = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
```
