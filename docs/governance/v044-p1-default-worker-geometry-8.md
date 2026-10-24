# V044 P1 — Default worker geometry (coordinated parallelism)

```text
STATUS = CLOSED_VERIFIED_REAL_PRODUCTION
CASE = G1
WIP = V044_P1_DEFAULT_WORKER_GEOMETRY_8
ROOT = PARALLELISM_UNDERUTIL
WIP_LIMIT = 1
P4_STARTED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

BASE_HEAD = 06742e1bdc67b5ed86ef95750399e9edad16d646
BASE_TREE = 5818095793373bd322ac13a2b3ebcf32661941e1
BASE_BINARY_SHA256 = 09301dc867a4161ebf1da24635f97df14acffcc25f8211b1e475b97b6defe36f

PRODUCT_COMMIT = ade997a362e60d1dc6eafc984da454930600845a
PRODUCT_TREE = 4ae6ffc7528d0881be23fcac246c345dcb9a1606
PRODUCT_BINARY_SHA256 = e28c0abe18587f2d911d492be64cbec42372383e011417a6c578be52da5ba459

EVIDENCE_NETCUP =
  /root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p1-default-worker-geometry-8-20260822T035125Z/
HOST = Netcup amd64 (netcup-bench)
IMAGE = v044p1-geom8-exyonq
```

## Historical correction (permanent)

Historical 4→8 experiments that raised only `EXYONQ_WORKER_THREADS` /
`EXYONQ_ACCEPT_WORKERS` were **not** authoritative for Cap067 scaling because
`EXYONQ_EPOLL_POOL_THREADS` stayed at the old default **4**.

Cap067 pool size is effectively:

```text
min(resolved_epoll_pool, accept_workers)
```

AB3 proved the matched geometry **8/8/8** (WT/AW/EPOLL_POOL) under the seal
binary. This WIP converts that causal finding into **production defaults**
without hard-coding literal 8 on every host.

## Design (ADR-021)

```text
DESIGN_PROBLEM =
  Independent defaults allowed Tokio/accept to scale while Cap067 silently
  remained at 4, under-delivering available CPU parallelism.

RESPONSIBILITIES_BEFORE =
  CLI: EXYONQ_WORKER_THREADS unwrap_or(4)
  Accept: EXYONQ_ACCEPT_WORKERS unwrap_or(4) / auto
  Cap067: EXYONQ_EPOLL_POOL_THREADS unwrap_or(4).clamp(1,8)
  Bench Dockerfile: ENV pins WT=4 AW=4

PATTERN_SELECTED = C — single DEFAULT_RUNTIME_PARALLELISM SoT
  default_runtime_parallelism() = available_parallelism().clamp(1, 16)
  → Tokio / accept / Cap067 when env unset
  Cap067 unset follows accept (cannot stay at 4 while accept scales)

PATTERN_REJECTED =
  A fixed-8 everywhere — oversubscribes 2/4-CPU hosts and containers
  B quota-only without SoT — still leaves three knobs uncoordinated
  D larger topology framework — out of WIP scope

CPU_TOPOLOGY_SOURCE = std::thread::available_parallelism()
CPUSET_AWARE = YES   # Linux affinity / cpuset
CGROUP_QUOTA_AWARE = NO  # available_parallelism does not read cpu.max quota
DEFAULT_WORKER_POLICY = detect_and_clamp
MIN_WORKERS = 1
MAX_WORKERS = 16
OVERRIDE_SEMANTICS =
  explicit positive env honored (clamped);
  ACCEPT_WORKERS=auto → detect;
  EPOLL_POOL unset → accept;
  EPOLL_POOL explicit may not exceed accept (min)
```

Implementation: `core/src/server/runtime_parallelism.rs` (+ CLI / Cap067 wiring).
Unit matrix D1–D5 covered in that module’s tests.

Startup proof (Netcup, nproc=8, worker env unset):

```text
workers=8
accept_workers=8 epoll_pool_threads=8
```

## Gates vs AB3 / old default

| Gate | Result |
|------|--------|
| P1 RPS vs old seal (141938) | **+8.54%** (≥ +5%) |
| P1 RPS vs AB3 A0 (139350) | **+10.56%** |
| P1 P99 vs old seal (1.81→1.71) | **−5.5%** (no >+5% regress) |
| P3 RPS vs prior seal (8228→8683) | **+5.54%** (no >+3% regress) |
| Correctness/security regressions | **none observed** |

```text
CASE = G1
ROLLBACK = NOT_REQUIRED
```

## Same-session three-way (geom8 image)

Contract: Static keepalive HTTP/1.1; cpuset 0–7; RAW WAF OFF; rewrk −c 100 −t 2;
20s warmup / 30s measure / 5 reps; worker env **unset** (product defaults).

### P1 — 1 KiB

| | ExyonQ | NGINX | OLS |
|--|-------:|------:|----:|
| RPS median | 154062 | **168895** | 133498 |
| core-ms/req | 0.130 | 0.161 | **0.122** |
| RSS MiB | 13.86 | **8.42** | 13.97 |
| P50 / P95 / P99 | 0.79 / 1.29 / 1.71 | **0.73 / 1.15 / 1.55** | 0.98 / 1.50 / 1.81 |

```text
EXYONQ_VS_NGINX_RPS_PERCENT ≈ −8.78%   # was ≈ −12.5% on prior seal
EXYONQ_VS_OLS_RPS_PERCENT ≈ +15.4%
P1_ENGINEERING_BASELINE = CLOSED
P1_PERFORMANCE_COMPETITIVENESS = MIXED
P1_COMPETITIVE_WIN = NOT_PROVEN
P1_MIGRATION_VALUE_VS_NGINX = NOT_PROVEN
P1_MIGRATION_VALUE_VS_OLS = PARTIAL
P1_MARKET_READINESS = NOT_READY
```

### P2 — 64 KiB

| | ExyonQ | NGINX | OLS |
|--|-------:|------:|----:|
| RPS median | **104041** | 90212 | 101791 |
| core-ms/req | **0.197** | 0.235 | 0.226 |
| RSS MiB | 21.64 | **13.14** | 19.27 |
| P50 / P95 / P99 | **1.20** / 1.87 / 2.41 | 1.29 / 2.03 / 2.50 | 1.23 / **1.82** / **2.27** |

```text
P2_RPS_EFFECT ≈ +6.2% vs prior ExyonQ seal median (97932)
P2_COMPETITIVE_WIN = NOT_PROVEN   # still loses P95/P99 to OLS and RSS to NGINX
P2_MIGRATION_VALUE = PARTIAL
P2_MARKET_READINESS = NOT_READY
```

### P3 — 1 MiB

| | ExyonQ | NGINX | OLS |
|--|-------:|------:|----:|
| RPS median | 8683 | 8594 | **8709** |
| core-ms/req | **1.146** | 1.151 | 1.207 |
| RSS MiB | 71.95 | **58.82** | 65.26 |
| P50 / P95 / P99 | **13.95 / 19.70 / 25.37** | 14.37 / 20.28 / 25.73 | 16.92 / 25.42 / 31.77 |

```text
P3_RPS_EFFECT ≈ +5.5% vs prior ExyonQ seal
P3_COMPETITIVE_WIN_AFTER = YES   # package preserved (CPU+latency wins; near-parity RPS)
```

**Meter note:** docker-stats core-ms/req in this harness is same-session comparable
across ExyonQ/NGINX/OLS. Absolute values are **not** interchangeable with the
prior live-3way seal’s smaller core-ms scale without renormalization.

## Architecture fitness consequence

```text
ARCHITECTURE_FITNESS = TARGETED_ARCHITECTURAL_EVOLUTION
PARALLELISM_UNDERUTIL_STATUS = CLOSED_FIXED_AND_REBENCHMARKED
ROOTS_RWLOCK_STATUS = REJECTED_NOT_MATERIAL   # preserved from AB3
HANDLE_REGISTRY_STATUS = REJECTED_NOT_MATERIAL
STAGE_DEPTH_STATUS = NOT_PRIMARY_THIS_SESSION
P4_STARTED = NO
```

Do **not** auto-start stage-depth, io_uring, or thread-per-core work.

## Logic audit (scoped)

```text
LOGIC_AUDIT_AGENT = exyonq-logic-auditor
LOGIC_AUDIT_STATUS = PASS_WITH_NONBLOCKING_P3
OPEN_LOGIC_P0 = 0
OPEN_LOGIC_P1 = 0
OPEN_PROVEN_PRODUCT_DEFECTS = 0
```

Non-blocking P3 notes: invalid-env soft-fallback; Tokio clamp alignment to 16.

## Terminal

```text
V044_P1_DEFAULT_WORKER_GEOMETRY_8_STATUS = CLOSED_VERIFIED_REAL_PRODUCTION
CASE = G1
STOP = YES
NEXT_PERF_ROOT = OWNER_REVIEW_ONLY
```
