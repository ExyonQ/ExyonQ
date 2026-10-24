# V044 P1 — Fixed-stage & parallelism A/B

```text
STATUS = COMPLETE
CASE = AB3
WIP = V044_P1_FIXED_STAGE_AND_PARALLELISM_A_B
MODE = READ_ONLY_MEASUREMENT_AB
PRODUCT_MUTATION = NO
ARCHITECTURE_MUTATION = NO
P4_STARTED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

PRODUCT_AUTHORITY_HEAD = 06742e1bdc67b5ed86ef95750399e9edad16d646
PRODUCT_AUTHORITY_TREE = 5818095793373bd322ac13a2b3ebcf32661941e1
PRODUCT_BINARY_SHA256 = 09301dc867a4161ebf1da24635f97df14acffcc25f8211b1e475b97b6defe36f
IMAGE = v044p1auth-exyonq  # sealed SHA (v044p1auth-clean-exyonq had drifted)

EVIDENCE =
  /root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p1-fixed-stage-parallelism-ab-20260822T024222Z/
HOST = Netcup amd64 (netcup-bench)
CONTRACT = P1 RAW WAF OFF; cpuset 0-7; CpuQuota=0; rewrk -c 100 -t 2; warmup 20s; measure 30s; 5 reps
```

## Method note (historical A/B invalidated)

Cap067 keepalive pool size is:

```text
min(EXYONQ_EPOLL_POOL_THREADS.unwrap_or(4).clamp(1,8), EXYONQ_ACCEPT_WORKERS)
```

Prior residual discriminator 4→8 A/B raised `EXYONQ_WORKER_THREADS` / `EXYONQ_ACCEPT_WORKERS`
but left `EXYONQ_EPOLL_POOL_THREADS` empty → Cap067 stayed at **4**. That session’s
`WORKER_TOPOLOGY_CAUSAL_SIGNAL=INCONCLUSIVE` is **not** authority for this WIP.

This A/B sets `EPOLL_POOL_THREADS` explicitly.

## Arms

| Arm | WT | AW | EPOLL_POOL | Intent |
|-----|---:|---:|-----------:|--------|
| A0 | 4 | 4 | 4 | Canonical seal geometry |
| A1 | 8 | 8 | 8 | Full scale (Tokio+accept+Cap067) |
| A2 | 6 | 6 | 6 | Intermediate |
| A_TOKIO | 8 | 4 | 4 | Tokio-only increase |
| A_CAP067 | 4 | 8 | 8 | Cap067/accept pool without Tokio scale |

## A/B-A results (P1)

| Arm | Median RPS | CV | CPU% med | core-ms/req | P50 | P95 | P99 | RSS MiB |
|-----|-----------:|---:|---------:|------------:|----:|----:|----:|--------:|
| A0 | 139350 | 0.083 | 221.5 | 0.1272 | 0.83 | 1.32 | 1.68 | 10.2 |
| **A1** | **153789** | **0.058** | 254.5 | 0.1324 | 0.89 | **1.21** | **1.51** | 11.8 |
| A2 | 150751 | 0.069 | 245.4 | 0.1302 | 0.81 | 1.22 | 1.57 | 12.1 |
| A_TOKIO | 133384 | 0.155 | 234.8 | 0.1408 | 0.86 | 1.36 | 1.72 | 12.2 |
| A_CAP067 | 146734 | 0.102 | 244.3 | 0.1332 | 0.94 | 1.46 | 1.89 | 10.7 |

### Deltas vs A0

| Arm | RPS | core-ms | P50 | P95 | P99 |
|-----|----:|--------:|----:|----:|----:|
| **A1** | **+10.36%** | +4.08% | +7.23% | **−8.33%** | **−10.12%** |
| A2 | +8.18% | +2.41% | −2.41% | −7.58% | −6.55% |
| A_TOKIO | −4.28% | +10.73% | +3.61% | +3.03% | +2.38% |
| A_CAP067 | +5.30% | +4.72% | +13.25% | +10.61% | +12.50% |

```text
IS_WORKER_COUNT_LIMITING_RPS = YES
DO_MORE_WORKERS_IMPROVE_RPS = YES          # A1 full geometry
DO_MORE_WORKERS_IMPROVE_P95_P99 = YES      # A1
DO_MORE_WORKERS_INCREASE_CPU_COST = YES_SMALL  # +4% core-ms/req
IS_CPU_CAPACITY_LEFT_IDLE = YES             # A0 ~3.0 CPUs utilized / 8; A1 ~3.4
IS_LOAD_BALANCE_UNEVEN = PARTIAL            # A0 load on 4 cores; A1 spreads to more PSRs
```

Busy-thread sample under load: A0 ≈4 hot `tokio-rt-worker`; A1 ≈8 hot workers.
A_TOKIO alone does **not** unlock RPS (slight loss). Cap067/accept without Tokio
unlocks some RPS but **hurts** tail latency. Matched 8/8/8 is the winning arm.

## A/B-B — first vs keepalive

Host→container IP wall times showed ~100 ms on every new TCP connect (artifact).
In-docker curl (bench-runner):

| Step | time_starttransfer |
|------|-------------------:|
| close (varies) | ~1.6–90 ms |
| ka1 (first on conn) | ~94 ms |
| ka2 / ka3 (reuse) | **~0.15–0.18 ms** |

Keepalive subsequent path is µs-class. First request / new connection pays divert or
setup; absolute first-req µs **not** cleanly bounded as product handoff cost (tooling
+ connection-setup confound).

```text
TOKIO_EPOLL_SEAM_COST = BOUNDED
  # subsequent KA << first; not proven primary P1 RPS root vs worker geometry
FIRST_REQUEST_COST = NOT_CLEANLY_ISOLATED_US
KEEPALIVE_REQUEST_COST = ~0.15–0.18 ms wall (in-docker curl reuse)
```

## A/B-C — Cap067 paid stages (perf sample, A0)

Self-percent sums for RwLock / Mutex / SendfileHandleRegistry / WAF symbols: **≈0%**
(below clean attribution in this sample). Hits show kernel sendfile + spin unlock on
RX path. Historical Root2 registry bypass ≈ −0.335% CPU remains falsified.

```text
ROOTS_RWLOCK_COST = BELOW_MEASUREMENT_RESOLUTION
ROOTS_RWLOCK_CONTENTION = NONE_OBSERVED
HANDLE_REGISTRY_MUTEX_COST = BELOW_MEASUREMENT_RESOLUTION
HANDLE_REGISTRY_ROOT = REJECTED_NOT_MATERIAL
WAF_OFF_GATE_COST = BELOW_MEASUREMENT_RESOLUTION
GENERATION_GATE_COST = BELOW_MEASUREMENT_RESOLUTION / CLOSED_NOT_CURRENT_TARGET
ACCESS_HOOK_COST = BELOW_MEASUREMENT_RESOLUTION
FSM_STAGE_DEPTH_COST = NOT_ISOLATED_AS_DOMINANT_VS_PARALLELISM
```

## P3 short sanity (relative only; not full rebaseline)

| Arm | RPS | P50 | P95 | P99 |
|-----|----:|----:|----:|----:|
| A0 | 5640 | 18.55 | 26.85 | 37.45 |
| A1 | 6209 | 20.63 | 28.66 | 37.89 |

```text
P3_RPS_REGRESSION_PERCENT ≈ -10.1%  # negative = improvement on this short window
P3_P95_REGRESSION_PERCENT ≈ +6.7%
P3_P99_REGRESSION_PERCENT ≈ +1.2%
P3_CPU_REGRESSION_PERCENT = NOT_RELIABLE  # docker-stats ~3% on short window
DO_MORE_WORKERS_REGRESS_P3 = NOT_OBSERVED_AS_RPS_REGRESSION / TARGETED_SANITY_ONLY
```

Absolute P3 RPS below seal (~8228) — short measure / not competitive rebaseline.

## Root ledger

### PARALLELISM_UNDERUTIL

```text
ROOT_ID = PARALLELISM_UNDERUTIL
EVIDENCE_STRENGTH = STRONGLY_SUPPORTED
MEASURED_CURRENT_COST = A0 under-delivers vs A1 on same binary/host/workload
MEASURED_RECOVERABLE_COST = RPS +10.4%; P95 -8.3%; P99 -10.1%; core-ms +4.1%
RPS_EFFECT = +10.36% (A1 vs A0)
P50_EFFECT = +7.2% (slight worse)
P95_EFFECT = -8.3%
P99_EFFECT = -10.1%
CPU_EFFECT = +4.1% core-ms/req; CPU% 221→254
MEMORY_EFFECT = +1.6 MiB RSS
P2_APPLICABILITY = HIGH   # fixed concurrency/geometry, not payload-proportional
P3_REGRESSION_RISK = LOW_BOUNDED  # short sanity: RPS up; P95 mild up
IMPLEMENTATION_COMPLEXITY = LOW   # runtime env / compose defaults only
CORRECTNESS_RISK = LOW
SECURITY_RISK = NONE
RECOMMENDATION = YES_AS_FUTURE_CONFIG_WIP_NOT_IMPLEMENTED_HERE
```

### TOKIO_EPOLL_FIRST_REQUEST_SEAM

```text
EVIDENCE_STRENGTH = BOUNDED
MEASURED_RECOVERABLE_COST = NOT_BOUNDED_AS_P1_RPS_PRIMARY
RECOMMENDATION = NO_PRODUCT_WIP_NOW
```

### ROOTS_RWLOCK / HANDLE_REGISTRY / WAF_OFF / GENERATION / ACCESS_HOOK

```text
EVIDENCE_STRENGTH = REJECTED_NOT_MATERIAL or BELOW_RESOLUTION
RECOMMENDATION = NO
HANDLE_REGISTRY_ROOT = REJECTED_NOT_MATERIAL  # preserves Root2 falsification
```

### FSM_STAGE_DEPTH

```text
EVIDENCE_STRENGTH = HYPOTHESIS / NOT_PRIMARY_THIS_SESSION
RECOMMENDATION = NO_NOW
  # architecture fitness still allows later B/F evolution; this A/B did not
  # prove stage micro-costs as recoverable P1 multi-metric root
```

## Expected product value gate (for future config WIP)

```text
CAUSAL_EVIDENCE_STRENGTH = STRONGLY_SUPPORTED
EXPECTED_RPS_GAIN_PERCENT ≈ +8 to +12 (bounded by this A/B median +10.4)
EXPECTED_P50_GAIN_PERCENT ≈ -3 to +8 (mixed; A1 P50 slightly worse)
EXPECTED_P95_GAIN_PERCENT ≈ -5 to -10
EXPECTED_P99_GAIN_PERCENT ≈ -8 to -12
EXPECTED_CPU_EFFECT_PERCENT ≈ +3 to +6 core-ms/req
EXPECTED_P2_EFFECT = HIGH applicability; expect similar concurrency unlock
EXPECTED_P3_REGRESSION_RISK = LOW; require short P3 sanity before accept
EXPECTED_USER_VALUE = material for P1 RPS + tail; may not alone flip COMPETITIVE_WIN vs NGINX
MINIMUM_ACCEPTABLE_GAIN = RPS ≥ +5% AND (P95≤0% OR P99≤0%) vs A0 same-session
RISK_BUDGET = LOW (env default only; no Cap067 rewrite)
ROLLBACK_CRITERIA = revert env to 4/4/4 if RPS < +3% or P99 regresses > +5% or P3 RPS regresses > +3%
EXPECTED_COMPLEXITY_DELTA = LOW
```

## Decision

```text
CASE = AB3
STATUS = COMPLETE_PARALLELISM_ROOT_PROVEN
RECOMMENDED_ROOT = PARALLELISM_UNDERUTIL
NEXT_PRODUCT_WIP_PROPOSAL =
  SUPERSEDED — executed as V044_P1_DEFAULT_WORKER_GEOMETRY_8
  STATUS = CLOSED_VERIFIED_REAL_PRODUCTION (CASE G1)
  DOC = docs/governance/v044-p1-default-worker-geometry-8.md
  # Product default: coordinated available_parallelism SoT (not blind hard-code 8)
  # Rebenchmarked P1/P2/P3 on Netcup; NGINX RPS gap narrowed (~−8.8%) but
  # P1 COMPETITIVE_WIN remains NOT_PROVEN.

P4_STARTED = NO
PRODUCT_CODE_CHANGED = NO  # this A/B measurement WIP only
```

Follow-on product change and competitive scoreboard refresh live in
`docs/governance/v044-p1-default-worker-geometry-8.md` and
`docs/benchmarks/v044-p1-p14-competitive-dashboard.json`.
