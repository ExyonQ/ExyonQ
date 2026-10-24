# V044 — Architecture fitness review before P4

```text
STATUS = COMPLETE
WIP = V044_ARCHITECTURE_FITNESS_REVIEW_BEFORE_P4
MODE = READ_ONLY_ARCHITECTURAL_DIFFERENTIAL_ANALYSIS
PRODUCT_MUTATION = NO
ARCHITECTURE_REWRITE = NOT_AUTHORIZED
P4_STARTED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

PRODUCT_AUTHORITY_HEAD = 06742e1bdc67b5ed86ef95750399e9edad16d646
PRODUCT_AUTHORITY_TREE = 5818095793373bd322ac13a2b3ebcf32661941e1
GOVERNANCE_HEAD_AT_REVIEW = 2e515cdf4771fd8027e6f334e4569c25da5fb71b

EVIDENCE =
  P1 seal three-way + cgroup + deficit attribution (PSS / underutilization)
  P2 64 KiB competitive baseline
  P3 1 MiB competitive baseline (COMPETITIVE_WIN=YES)
  Cap067 path map (code): epoll_worker / epoll_session / wire_dispatch / Root1–3 / FD cache
```

## Disproven / not supported

```text
RUST_IS_INTRINSICALLY_TOO_SLOW = NOT_SUPPORTED
  # P3 competitive win on CPU + P50/P95/P99 with near-parity RPS
GLOBAL_ARCHITECTURE_IS_INTRINSICALLY_UNCOMPETITIVE = NOT_SUPPORTED
  # same — architecture can win at large static payload
```

```text
SMALL_MEDIUM_REQUEST_ARCHITECTURE_FITNESS = NOT_PROVEN
  # P1 COMPETITIVE_WIN=NOT_PROVEN; P2 MIXED/NOT_PROVEN
```

## Cross-profile matrix (authoritative seals)

| Axis | P1 (1 KiB) | P2 (64 KiB) | P3 (1 MiB) |
|------|------------|-------------|------------|
| RPS vs NGINX | LOSS (−12.5%) | slight WIN (+1.9%) | slight WIN (+1.0%) |
| RPS vs OLS | slight WIN | LOSS (−3.6%) | near PARITY (−0.2%) |
| CPU/req | WIN vs NGINX; LOSS vs OLS | WIN both | WIN both |
| Memory RSS | LOSS vs NGINX (~2×) | LOSS modest | NEAR NGINX (+2.7%) |
| P50/P95/P99 | LOSS vs NGINX | NEAR / slight LOSS | **WIN both** |
| Stability (RPS CV) | mixed | LOSS (higher CV) | **WIN** |
| Competitive win | NOT_PROVEN | NOT_PROVEN | **YES** |

## Cost model (docker-stats core-ms → µs/req)

Measured TOTAL (not invented):

| Profile | bytes | ExyonQ µs | NGINX µs | OLS µs |
|---------|------:|----------:|---------:|-------:|
| P1 | 1024 | 11.52 | 13.69 | 10.51 |
| P2 | 65536 | 19.34 | 20.47 | 20.59 |
| P3 | 1048576 | 101.51 | 107.76 | 111.49 |

Linear fit `TOTAL_us ≈ F + k·bytes` using **P1+P3 only** (P2 as check):

| Server | F (µs) fixed estimate | k (µs/KiB) | P2 pred vs actual |
|--------|----------------------:|-----------:|-------------------|
| ExyonQ | **11.43** | 0.0859 | 17.1 vs 19.3 (−12%) |
| NGINX | **13.60** | 0.0898 | 19.5 vs 20.5 (−5%) |
| OLS | **10.41** | 0.0964 | 16.7 vs 20.6 (−19%) |

```text
EXYONQ_FIXED_COST_ESTIMATE ≈ 11.4 µs/req   # docker-stats core-time; BOUNDED
NGINX_FIXED_COST_ESTIMATE  ≈ 13.6 µs/req
OLS_FIXED_COST_ESTIMATE    ≈ 10.4 µs/req
```

Cgroup companion (separate meter; includes more system time):

| | P1 total µs | P2 total µs | P3 total µs |
|--|------------:|------------:|------------:|
| ExyonQ | 32.62 | 26.46 | 157.59 |
| NGINX | 39.27 | 29.49 | 173.61 |
| OLS | 31.00 | 33.54 | 176.22 |

### Critical paradox (must not be misread)

On P1, ExyonQ already has **lower** CPU/request than NGINX (−15.9% docker-stats)
but **lower RPS** and **higher latency**.

Therefore:

```text
P1_LOSS_IS_NOT_EXPLAINED_BY_HIGHER_CPU_PER_REQUEST_VS_NGINX = TRUE
P1_LOSS_CLASS = UNDERUTILIZATION_PARALLELISM_QUEUEING_PLUS_FIXED_STAGE_DEPTH
  # prior R1_PARALLELISM_UNDERUTIL STRONGLY_SUPPORTED; stage depth from code STRONGLY_SUPPORTED
```

Payload amortization finding:

```text
PAYLOAD_AMORTIZATION_FINDING =
  As bytes grow, kernel TX (sendfile/system) dominates TOTAL_REQUEST_COST.
  ExyonQ’s Cap067 sendfile path is competitive once fixed stages are a smaller
  fraction of wall time → P3 latency+CPU win with RPS near parity.
  Small requests expose fixed stage depth + parallelism ceiling.
```

Approximate decomposition (qualitative; not fabricated microbenchmarks):

```text
TOTAL_REQUEST_COST =
  FIXED_REQUEST_COST      # parse, route, WAF atomic gate, match, locks, FSM setup
  + PAYLOAD_DEPENDENT_COST  # sendfile iterations ∝ size
  + KERNEL_TX_COST          # dominant at P3 (cgroup system µs)
  + SCHEDULING_COST         # wakeups / underutil (P1 RPS paradox)
  + MEMORY/ALLOC_COST       # process image (P1 RSS); not primary P3 differentiator
  + FEATURE/CONTROL_COST    # reload generation checks; WAF OFF = atomic only
```

---

## Layer audits

### 1. Language / codegen

```text
RUST_LANGUAGE_CHOICE = NOT_CAUSAL   # for global uncompetitiveness
  # P3 proves Rust+current stack can win CPU and latency
  # Code size / binary RSS still contribute to MEMORY gap (PSS attribution)
```

No evidence that bounds-check or enum dispatch alone explains P1 RPS loss vs NGINX
while CPU/request is already better.

### 2. Runtime execution model

```text
RUNTIME_MODEL_FINDING =
  Hybrid: Tokio owns accept + first-request wire divert;
  Cap067 OS threads own keepalive sendfile epoll FSM.
  P1 compose: EXYONQ_WORKER_THREADS=4, ACCEPT_WORKERS=4, keepalive pool=4.
```

```text
IS_HYBRID_TOKIO_PLUS_EPOLL_COSTLY = PARTIAL / STRONGLY_SUPPORTED_FOR_FIRST_REQUEST
  # First GET pays Tokio→epoll handoff (dup/enqueue/epoll_ctl ADD).
  # Subsequent keepalive GETs are mostly pure Cap067.
  # Not proven as sole P1 root; contributes to fixed stage depth.
```

### 3. Request path architecture (WAF OFF Cap067 hit)

Stages still paid (code map):

| Stage | Classification |
|-------|----------------|
| epoll_wait readiness | ESSENTIAL |
| nonblocking header read | ESSENTIAL |
| header safety / generation / drain gates | CONTRACT_REQUIRED |
| WAF evaluate entry | SECURITY_REQUIRED interface; body = atomic early-out when OFF |
| runtime TLS pin | ESSENTIAL (Root1) |
| roots RwLock read | CONTRACT_REQUIRED today; candidate for stricter locality |
| FD cache try_hit | ESSENTIAL (proven CPU win) |
| SendfileHandleRegistry Mutex issue+take | OPTIONAL_BUT_CURRENTLY_PAID (Root2 reverted) |
| session begin/pump sendfile | ESSENTIAL |
| access hook notify | OPTIONAL_BUT_CURRENTLY_PAID (observability) |
| keepalive rearm | ESSENTIAL |

```text
STATIC_FAST_PATH_FINDING =
  Cap067 is deeper than a single-worker nginx sendfile loop
  (Tokio divert seam + opaque handle registry + dual-interest FSM).
  Not a full general-purpose Hyper path on hit, but not minimal either.
```

### 4. Worker / parallelism

```text
EPOLL_MODEL_FINDING =
  4 Cap067 keepalive workers vs NGINX/OLS ~8 workers on same 8-CPU cpuset.
  P1: ExyonQ CPU% ≈163 vs NGINX ≈222 while RPS lower → underutilization.
  Historical workers=4 vs 8 A/B INCONCLUSIVE (pre-current seal) —
  repair gain UNBOUNDED → no product worker change authorized from that alone.
```

### 5. Syscall geometry

```text
Post-FD-reuse Cap067: openat2≈0, sendfile≈1, newfstatat≈1 (P1 sanity).
Instructions/branches/off-CPU on current seal three-way = NOT_MEASURED.
Historical residual docs are SUPERSEDED for primary ranking.
```

### 6. Memory / process image

```text
MEMORY_MODEL_FINDING =
  P1 PSS gap ≈9.5 MiB vs NGINX dominated by fat binary Private_Clean + anon runtime.
  FD cache NOT_PRIMARY.
IS_MEMORY_FOOTPRINT_STRUCTURAL = PARTIAL
  # process-image / binary size is architectural to a Rust monolith binary;
  # not proven to cause P1 RPS loss (CPU/req already better than NGINX).
```

### 7. Data plane vs control plane

```text
WAF OFF: inspection not run; atomic gate remains.
Reload generation atomics remain on path (correctness).
SendfileHandleRegistry + roots RwLock remain hot shared sync.
COLD_CONTROL_PLANE + MINIMAL_HOT_DATA_PLANE = PARTIALLY_SATISFIED
```

---

## Architecture options (no implementation)

| Option | Evidence | P1 effect | P2 effect | P3 effect | Complexity | Risk | Recommend |
|--------|----------|-----------|-----------|-----------|------------|------|-----------|
| A KEEP CURRENT | P3 already wins | none | none | preserve | Low | Low | NO as sole strategy if migration goal includes P1/P2 |
| B TARGETED SMALL-REQ FAST PATH | stage depth + locks | potentially multi-metric | likely help | low regression if invariants kept | Med | Med | **YES (direction)** |
| C SEPARATE DATA-PLANE ENGINE | insufficient | unknown | unknown | high risk | High | High | NO |
| D THREAD-PER-CORE / SHARDED | underutil STRONG; A/B inconclusive | unknown | unknown | unknown | High | Med–High | NOT_YET |
| E io_uring | speculative | unknown | unknown | unknown | High | High / portability | **NOT_JUSTIFIED_YET** |
| F Stricter worker-local ownership, fewer handoffs | registry Mutex, Tokio divert | plausible | plausible | low | Med | Med | YES as subset of B |
| G MAJOR REWRITE | barred by P3 win | — | — | — | Very High | Very High | **NO** |

### io_uring

```text
IO_URING_RECOMMENDATION = NOT_JUSTIFIED_YET
  # No current-seal proof that epoll/sendfile kernel path is the P1 bottleneck
  # vs parallelism/stage depth. Multishot/SQPOLL/send_zc remain HYPOTHESIS.
```

### Thread-per-core

Fits goals of affinity and no cross-core ownership **in principle**, but:

- current Cap067 already worker-local for sessions/FD cache;
- historical scale A/B did not prove 4→8 unlock;
- recommending TPC without fresh measurement A/B = HYPOTHESIS.

```text
WOULD_THREAD_PER_CORE_HELP = UNKNOWN / NOT_PROVEN
```

---

## Architecture decision table (required)

| ARCH_OPTION | EVIDENCE | P1_GAIN | P2_GAIN | P3_EFFECT | CPU | MEM | TAIL | CX | RISK | PORT | MAINT | REC |
|-------------|----------|---------|---------|-----------|-----|-----|------|----|------|------|-------|-----|
| CURRENT | PROVEN P3 win | 0 | 0 | keep | — | — | — | L | L | L | L | NO alone |
| TARGETED_FAST_PATH | STRONGLY_SUPPORTED | multi-metric **if** fixed stages/parallelism addressed | likely | preserve | ↓ fixed | maybe ↓ | ↓ P1 | M | M | L | M | **YES** |
| THREAD_PER_CORE | HYPOTHESIS | unbound | unbound | risk | ? | ? | ? | H | H | L | H | NO now |
| IO_URING | HYPOTHESIS | unbound | unbound | risk | ? | ? | ? | H | H | H | H | NO |
| SEPARATE_DATA_PLANE | HYPOTHESIS | unbound | unbound | high risk | ? | ? | ? | VH | VH | M | VH | NO |
| MAJOR_REWRITE | DISPROVEN need | — | — | — | — | — | — | VH | VH | VH | VH | **NO** |

---

## Final Q&A

```text
IS_RUST_THE_PROBLEM = NO
IS_TOKIO_THE_PROBLEM = PARTIAL   # accept/first-request seam; not whole stack
IS_CURRENT_EPOLL_MODEL_THE_PROBLEM = PARTIAL   # Cap067 sound at P3; small-req depth+workers
IS_HYBRID_TOKIO_PLUS_EPOLL_COSTLY = YES_FOR_FIRST_REQUEST / BOUNDED
IS_FIXED_PER_REQUEST_OVERHEAD_TOO_HIGH = YES_RELATIVE_TO_SMALL_PAYLOAD_WALLTIME
  # Note: CPU/req vs NGINX is NOT higher on P1 — “too high” means stage depth
  # relative to 1 KiB work and parallelism delivery, not raw core-ms vs NGINX.
IS_MEMORY_FOOTPRINT_STRUCTURAL = PARTIAL
WOULD_THREAD_PER_CORE_HELP = UNKNOWN
WOULD_IO_URING_HELP = NOT_JUSTIFIED_YET
DO_WE_NEED_A_NEW_DATA_PLANE_ARCHITECTURE = NO
DO_WE_NEED_A_FULL_REWRITE = NO
```

## Verdict

```text
ARCHITECTURE_FITNESS = TARGETED_ARCHITECTURAL_EVOLUTION

PRIMARY_STRUCTURAL_FINDING =
  Small/medium static loses commercially due to fixed stage depth + parallelism/
  underutilization; large static amortizes fixed cost and Cap067 sendfile wins.

FIXED_REQUEST_COST_FINDING =
  Linear F≈11–14 µs (docker-stats) across servers; ExyonQ F not worse than NGINX.
  Wall-clock/RPS still hurt by deeper pipeline + lower effective concurrency.

PAYLOAD_AMORTIZATION_FINDING =
  PROVEN by P1→P3 progression (competitive win appears at 1 MiB).

RECOMMENDED_ARCHITECTURE_DIRECTION =
  OPTION B/F still valid long-term; immediate highest-EV root is matched worker
  geometry (env defaults 8/8/8), not stage-mutex product surgery.
  Preserve WAF/reload/security/observability contracts.

EXPECTED_PRODUCT_VALUE =
  Improve P1/P2 migration case without sacrificing P3 large-file win.

EXPECTED_P1_EFFECT = A1 geometry +10.4% RPS, P95 −8.3%, P99 −10.1% (measured A/B)
EXPECTED_P2_EFFECT = HIGH applicability for same concurrency unlock (not re-run)
EXPECTED_P3_EFFECT = short sanity: RPS improved; P95 mild +; full rebaseline if defaults change

NEXT_WIP_PROPOSAL =
  SUPERSEDED_BY_COMPLETED_PRODUCT_WIP
  # V044_P1_FIXED_STAGE_AND_PARALLELISM_A_B = COMPLETE CASE AB3
  # V044_P1_DEFAULT_WORKER_GEOMETRY_8 = CLOSED_VERIFIED_REAL_PRODUCTION CASE G1
  # Doc: docs/governance/v044-p1-default-worker-geometry-8.md
  # PARALLELISM_UNDERUTIL = CLOSED_FIXED_AND_REBENCHMARKED
  # Do NOT auto-start P4 or stage-depth optimization.

P4_STARTED = NO
```

## Bars check

```text
MAJOR_REARCHITECTURE_JUSTIFIED = NO
  # fails criteria 1–5: P3 win; deficits not proven unfixable by bounded change
TARGETED_ARCHITECTURAL_EVOLUTION = YES (leading, evidence-bound)

POST_AB_UPDATE_20260822:
  WOULD_THREAD_PER_CORE_HELP remains UNKNOWN (not required for AB3 config gain)
  WOULD_IO_URING_HELP remains NOT_JUSTIFIED_YET
  FIXED_STAGE micro-roots BELOW_RESOLUTION / REJECTED_NOT_MATERIAL this session
  PARALLELISM_UNDERUTIL = STRONGLY_SUPPORTED (fresh A/B on sealed binary)

POST_GEOMETRY8_UPDATE_20260822:
  PARALLELISM_UNDERUTIL = CLOSED_FIXED_AND_REBENCHMARKED
  P1 RPS gain vs old default ≈ +8.5%; P3 competitive win preserved
  P1 COMPETITIVE_WIN still NOT_PROVEN (NGINX multi-metric package)
```
