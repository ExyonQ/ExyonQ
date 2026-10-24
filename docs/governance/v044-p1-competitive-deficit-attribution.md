# V044 P1 — Competitive deficit attribution

```text
STATUS = COMPLETE
CASE = C2
WIP = V044_P1_COMPETITIVE_DEFICIT_ATTRIBUTION
MODE = READ_ONLY_DIFFERENTIAL_CAUSAL_ANALYSIS
PRODUCT_MUTATION = NO
PRODUCT_CODE_CHANGED = NO
PERFORMANCE_REPAIR = NOT_AUTHORIZED
P2_EXECUTION = HOLD
P2_STARTED = NO
P3_STARTED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

GOVERNANCE_HEAD_AT_START = 225ce94cd74269329afcfbaee50d159f7e176472
GOVERNANCE_TREE_AT_START = 4707714fc58985de0774cca36af45a845021f27c
P1_PRODUCT_AUTHORITY_HEAD = 06742e1bdc67b5ed86ef95750399e9edad16d646
P1_PRODUCT_AUTHORITY_TREE = 5818095793373bd322ac13a2b3ebcf32661941e1
DOCKER_EXYONQ_BINARY_SHA256 = 09301dc867a4161ebf1da24635f97df14acffcc25f8211b1e475b97b6defe36f

EVIDENCE_SEAL =
  .exyonq-local/evidence/v044-p1-final-post-repair-rebaseline-20260821T191819Z/
EVIDENCE_ATTRIBUTION =
  .exyonq-local/evidence/v044-p1-competitive-deficit-attr-20260821T211202Z/
  (Netcup: /root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p1-competitive-deficit-attr-20260821T211202Z/)
```

## Axes preserved (do not collapse)

```text
P1_ENGINEERING_BASELINE = CLOSED
P1_COMPETITIVE_WIN = NOT_PROVEN
P1_MIGRATION_VALUE_VS_NGINX = NOT_PROVEN
P1_MIGRATION_VALUE_VS_OLS = NOT_PROVEN
P1_MARKET_READINESS = NOT_READY
P1_OPEN_ENGINEERING_CAUSAL_ROOTS = 0
```

This WIP does **not** reopen Root1–Root3, FD-reuse, or H1 miss hang.

## Phase 0 — evidence inventory

### EXISTING_EVIDENCE_USABLE

| Source | Covers |
|--------|--------|
| Seal three-way `threeway_summary.json` | RPS medians + reps + CV; docker-stats CPU%; core-ms/req; P50/P90/P95/P99; RSS avg/peak (docker MemUsage) |
| Seal cgroup companion | user/system/total µs/req (single window) |
| Cap067 FD/miss sanity on seal | FD-reuse still hit; not reopened |
| FD-reuse governance | FD_CACHE_MAX_ENTRIES=64/worker; expected P1 hot set tiny |
| Historical residual docs | Context only — **obsolete** for userspace 4.35 µs residual and SessionTable mutex as current primary |

### MISSING_REQUIRED_EVIDENCE (before this WIP)

| Gap | Action taken |
|-----|----------------|
| smaps / PSS / private vs shared | Collected on Netcup against seal binary (attribution evidence) |
| Thread topology under load | Collected (10 threads: 8 tokio-rt-worker + main + inotify) |
| FD count | Collected (~130 under load) |
| instructions/branches/cycles per req on **current** seal | Still **NOT_MEASURED** (not required to close CASE C2) |
| Latency series variance (multi-rep p50/p95/p99) | Still **NOT_MEASURED** (seal has one pct probe per server) |
| Off-CPU µs/req on current seal | Still **NOT_MEASURED** |

## Seal scoreboard deficits (authoritative competitive numbers)

| Metric | ExyonQ | NGINX | OLS | ExyonQ deficit |
|--------|--------|-------|-----|----------------|
| RPS | 141937.78 | 162268.07 | 138943.97 | vs NGINX **−12.53%**; vs OLS **+2.15%** |
| core-ms/req | 0.011519 | 0.013694 | 0.010512 | vs NGINX **−15.89%** (win); vs OLS **+9.58%** |
| RSS avg MiB (docker) | 18.519 | 8.480 | 16.388 | vs NGINX **+118.4%**; vs OLS **+13.0%** |
| P50 ms | 0.86 | 0.70 | 0.96 | vs NGINX **+22.9%** |
| P95 ms | 1.37 | 1.04 | 1.41 | vs NGINX **+31.7%** |
| P99 ms | 1.81 | 1.43 | 1.70 | vs NGINX **+26.6%**; vs OLS **+6.5%** |

```text
RPS_CV_EXYONQ = 0.133
RPS_CV_NGINX  = 0.195
RPS_CV_OLS    = 0.123
HOST_INTERFERENCE = OBSERVED (extra containers; disk ~95–96%)
```

RPS direction vs NGINX survives high CV (ExyonQ median below NGINX; overlapping tails exist — treat magnitude as **noisy**, sign as **stable enough** for attribution class). Latency deficits vs NGINX are single-probe — classify magnitude **BOUNDED**, not micromanage.

## Key contradiction: cheaper than NGINX per request, yet lower RPS and higher latency

Seal aggregate CPU% while serving:

| | CPU avg % | RPS | Interpretation |
|--|-----------|-----|----------------|
| ExyonQ | 163.5 | 141938 | Fewer cycles burned, fewer requests completed |
| NGINX | 222.2 | 162268 | Higher utilization, higher throughput |
| OLS | 146.1 | 138944 | Lowest CPU%, similar RPS to ExyonQ |

```text
ROOT_CLASS_RPS_LATENCY = CPU_UNDERUTILIZATION_OR_PARALLELISM_CEILING
CONFIDENCE = STRONGLY_SUPPORTED
```

Not “ExyonQ burns more CPU and is slower.” It burns **less** CPU than NGINX and still loses RPS/P50/P95/P99 → shared root is **capacity / parallelism / queueing**, not per-request CPU cost vs NGINX.

Attribution capture: 8 `tokio-rt-worker` threads present; historically Cap067 keepalive pin is **4 workers** (`EXYONQ_WORKER_THREADS=4` in P1 compose). Prior discriminator A/B for workers=4 vs 8 was **INCONCLUSIVE** on older residual WIP — **do not product-change workers without a fresh A/B on `06742e1b`**.

## Memory attribution (priority)

### Fair process accounting (this session, smaps)

Summing raw VmRSS across NGINX/OLS workers **double-counts shared libraries**. Use **PSS**:

| | sum Rss (misleading) | sum Pss (fair) | Private_Dirty |
|--|----------------------|----------------|---------------|
| ExyonQ (1 proc) | 21.9 MiB | **20.2 MiB** | 10.3 MiB |
| NGINX (master+8 workers) | 31.1 MiB | **10.7 MiB** | 5.1 MiB |
| OLS (main+workers) | 59.6 MiB | **15.1 MiB** | 8.3 MiB |

```text
MEMORY_GAP_EXYONQ_VS_NGINX_MIB ≈ 20.2 − 10.7 = 9.5  (PSS, this session)
```

### ExyonQ breakdown (mid_load)

```text
RSS_PRIVATE_DIRTY ≈ 10.3 MiB   # Anonymous / runtime
RSS_PRIVATE_CLEAN ≈ 9.5 MiB    # largely mapped binary (VmExe ≈ 15.6 MiB)
RSS_SHARED_CLEAN  ≈ 2.0 MiB
HEAP_[heap]       ≈ 1.1 MiB
STACK_[stack]     ≈ 0.1 MiB    # thread stacks mostly in anon, not [stack]
SO_MAPPINGS       ≈ 2.1 MiB
TEXTISH_EXYONQ    ≈ 10.1 MiB
FD_COUNT          ≈ 130
```

```text
EXYONQ_MEMORY_ATTRIBUTION =
  PRIMARY: fat process image (binary Private_Clean ~9–10 MiB) + anon runtime ~10 MiB
  SECONDARY: shared libs ~2 MiB; heap label small
  FD_CACHE: NOT_PRIMARY (≤64 entries/worker × 4 workers; P1 hot set ≪ cap; cannot explain ~10 MiB gap)

NGINX_MEMORY_ATTRIBUTION =
  Lean multi-process: per-worker Private_Dirty ~0.6 MiB; shared libc/.so amortized in PSS
  Seal docker MemUsage ~8.5 MiB under P1 load (authority for scoreboard)

OLS_MEMORY_ATTRIBUTION =
  PSS ~15 MiB; docker seal ~16.4 MiB — closer to ExyonQ than NGINX
```

### Seal docker RSS vs live cgroup drift

```text
SEAL_DOCKER_RSS_AUTHORITY = ExyonQ 18.5 / NGINX 8.5 / OLS 16.4 MiB
LIVE_CGROUP_UNDER_LOAD_THIS_SESSION ≈ ExyonQ 14.7 / NGINX 44.1 / OLS 17.6 MiB
```

NGINX live cgroup ~40+ MiB after long uptime is **HOST_STATE_DRIFT** — do **not** reverse the seal ranking from it. Controlled remasure after clean rival restart would be needed to refresh docker RSS; PSS still shows ExyonQ’s **structural** heavier single image.

```text
MEMORY_GAP_EXPLAINED_MIB ≈ 9–10   # PSS gap ≈ binary Private_Clean + excess anon vs NGINX PSS
MEMORY_GAP_UNEXPLAINED_MIB ≈ 0–2  # meter noise / docker vs PSS accounting
FD_CACHE_MEMORY_ROOT = NOT_PRIMARY
FD_CACHE_CURRENT_ENTRIES = low (P1 one hot path × workers)
FD_CACHE_MEMORY_ESTIMATE = << 1 MiB
FD_CACHE_RSS_SHARE_ESTIMATE = NOT_MATERIAL
```

## OLS CPU gap (current meters only)

Seal cgroup companion (not obsolete 4.35 µs user residual):

| | user µs/req | system µs/req | total |
|--|-------------|---------------|-------|
| ExyonQ | **6.661** | **25.959** | 32.619 |
| OLS | 9.339 | 21.657 | 30.996 |
| Gap (ExyonQ−OLS) | **−2.68** (ExyonQ better) | **+4.30** | +1.62 |

```text
USER_CPU_GAP = ExyonQ ahead (userspace not the OLS deficit)
SYSTEM_CPU_GAP = ExyonQ behind ≈ +4.3 µs/req  (primary OLS CPU deficit class)
EXYONQ_CPU_COST_VS_OLS_PERCENT (docker-stats) = +9.58%
CONFIDENCE_SYSTEM_CLASS = STRONGLY_SUPPORTED
INSTRUCTIONS_PER_REQ_CURRENT = NOT_MEASURED
```

## Shared roots map (8 deficits → few roots)

| Observed deficit | Shared root | Confidence |
|------------------|-------------|------------|
| RPS vs NGINX | R1 underutilization / parallelism ceiling | STRONGLY_SUPPORTED |
| P50/P95/P99 vs NGINX | R1 (queueing from limited parallel capacity) | STRONGLY_SUPPORTED |
| P99 vs OLS (small) | R1 partial + variance; may be **NOT_CAUSALLY_ESTABLISHED** alone | BOUNDED |
| RSS vs NGINX | R2 fat binary + anon runtime | STRONGLY_SUPPORTED |
| RSS vs OLS (modest) | R2 (ExyonQ PSS 20 vs OLS 15) | STRONGLY_SUPPORTED |
| CPU vs OLS | R3 system-CPU excess (~+4.3 µs/req) | STRONGLY_SUPPORTED |
| CPU vs NGINX | **not a deficit** (ExyonQ wins) | PROVEN |

## Ranked ledger

| ROOT_ID | METRIC(S) | EVIDENCE | CONF | CURRENT_COST | EXPECTED_RECOVERABLE_GAIN | USER_VALUE | COMPLEXITY | CORR/SEC RISK | RECOMMENDATION |
|---------|-----------|----------|------|--------------|---------------------------|------------|------------|---------------|----------------|
| R1_PARALLELISM_UNDERUTIL | RPS, P50, P95, P99 vs NGINX | Seal CPU%≪NGINX while RPS↓; 4 Cap067 workers vs 8 rival workers; prior worker A/B inconclusive | STRONGLY_SUPPORTED (class) / HYPOTHESIS (fix) | −12% RPS class; +23–32% latency class | **UNBOUNDED** without fresh A/B | High if unlocks multi-metric | Med–High | Med (topology) | **NO product yet — measurement A/B first** |
| R2_PROCESS_IMAGE_RSS | RSS vs NGINX/OLS | PSS + Private_Clean≈binary; heap small; FD cache not primary | STRONGLY_SUPPORTED | +~10 MiB PSS vs NGINX | Partial (strip/LTO/dep cut) — **unlikely to reach NGINX 8.5 MiB** | Medium alone; weak migration case | Med–High | Low–Med | **NO product — insufficient migration value alone** |
| R3_OLS_SYSTEM_CPU | CPU vs OLS | cgroup system +4.3 µs/req | STRONGLY_SUPPORTED | +9.6% core-ms | Unknown without syscall/perf on current binary | Low–Med | Med | Low | **NO product — need post-FD perf-stat first** |
| R4_FD_CACHE_RSS | RSS | cache bounds + FD count | PROVEN not primary | ≪1 MiB | N/A | N/A | N/A | N/A | **REJECT as RSS root** |
| R5_HOST_VARIANCE | all | CV 0.12–0.20; HOST_INTERFERENCE | PROVEN noise floor | blurs magnitudes | N/A | N/A | N/A | N/A | Do not optimize noise |

## Expected-value gate

```text
RECOMMENDED_ROOT = NONE
EXPECTED_RPS_GAIN_PERCENT = NOT_BOUNDED
EXPECTED_CPU_GAIN_PERCENT = NOT_BOUNDED
EXPECTED_MEMORY_GAIN_PERCENT = NOT_BOUNDED
EXPECTED_P50_GAIN_PERCENT = NOT_BOUNDED
EXPECTED_P95_GAIN_PERCENT = NOT_BOUNDED
EXPECTED_P99_GAIN_PERCENT = NOT_BOUNDED
EXPECTED_USER_VALUE = insufficient for product WIP
MINIMUM_ACCEPTABLE_GAIN = multi-metric material move toward COMPETITIVE_WIN
RISK_BUDGET = N/A (no repair)
ROLLBACK_CRITERIA = N/A
EXPECTED_COMPLEXITY_DELTA = N/A
NEXT_PRODUCT_WIP_PROPOSAL = NONE
```

Optional **measurement-only** follow-ups (owner must authorize separately; still `PRODUCT_MUTATION=NO`):

1. Clean restart of P1 trio → remasure docker RSS under seal contract (resolve NGINX 8.5 vs live 40 MiB drift).
2. Workers=4 vs 8 A/B on binary `09301dc8…` under RAW WAF-off (refresh inconclusive historical signal).
3. perf-stat / syscall differential for R3 system-CPU vs OLS on current binary.

## Migration answers

```text
WHY_NOT_MIGRATE_FROM_NGINX_TODAY =
  ExyonQ wins CPU/request but loses throughput, all listed latencies, and ~2× seal RSS.
  “Cheaper CPU” alone is not a migration case.

WHY_NOT_MIGRATE_FROM_OLS_TODAY =
  Only a small RPS edge; similar latency; higher RSS; worse CPU/request (system-side).
  No verified compensating product advantage counted.

WHAT_SINGLE_CHANGE_WOULD_MOST_IMPROVE_MIGRATION_VALUE =
  A proven multi-metric unlock of RPS+P95/P99 toward/above NGINX without correctness loss
  (likely R1 class) — but that change is NOT authorized and NOT gain-bounded yet.

CAN_P1_REALISTICALLY_REACH_COMPETITIVE_WIN = UNKNOWN
  Reasons: R1 fix unproven on current binary; R2 may permanently lose RSS vs NGINX
  (architectural binary size) while still allowing a Pareto win if RPS/latency/CPU package
  is compelling. Missing: fresh worker A/B + post-FD OLS system attribution.
```

## Case

```text
P1_COMPETITIVE_DEFICIT_ATTRIBUTION_STATUS = COMPLETE_NO_JUSTIFIED_PRODUCT_REPAIR
CASE = C2
```

Owner may accept P1 market limitation, authorize measurement follow-ups, or later reauthorize P2 — **P2 remains HOLD**.
