# V044 — Cross-profile performance root cause review (P1–P4)

```text
STATUS = COMPLETE
CASE = B
WIP = V044_CROSS_PROFILE_PERFORMANCE_ROOT_CAUSE_REVIEW_P1_P4
MODE = READ_ONLY_CAUSAL_ARCHITECTURE_PERFORMANCE_REVIEW
PRODUCT_MUTATION = NO
ARCHITECTURE_MUTATION = NO
P5_STARTED = NO
P5_TO_P14_FROZEN = YES
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

SOURCE_PRODUCT_HEAD = ade997a362e60d1dc6eafc984da454930600845a
SOURCE_PRODUCT_TREE = 4ae6ffc7528d0881be23fcac246c345dcb9a1606
PRODUCT_BINARY_SHA256 = e28c0abe18587f2d911d492be64cbec42372383e011417a6c578be52da5ba459
GOVERNANCE_HEAD_AT_REVIEW = eb689ebe2b4cdf7dd639f7d2c0c99fc63b117a5f
```

## Executive verdict

```text
V044_CROSS_PROFILE_PERFORMANCE_ROOT_CAUSE_REVIEW_P1_P4_STATUS = COMPLETE
CASE = B   # MULTIPLE_BOUNDED_ROOTS_IDENTIFIED
ARCHITECTURE_FITNESS_AFTER_P4 = TARGETED_ARCHITECTURAL_EVOLUTION
MAJOR_REARCHITECTURE_JUSTIFIED = NO
FULL_REWRITE_JUSTIFIED = NO
GLOBAL_MIGRATION_VALUE = NOT_PROVEN
MARKET_READINESS = NOT_READY
```

P1, P2, and P4 do **not** share one dominant systemic CPU root. They share a **memory footprint root** and a **small-payload fixed-cost sensitivity class**, but P4 adds a **separate, material proxy userspace tax** that static profiles do not exhibit. P3 remains decisive counter-evidence: the current architecture **can** be Pareto-competitive when kernel sendfile amortizes fixed stages.

---

## Phase 1 — Evidence inventory matrix

| EVIDENCE_ID | PROFILE | SOURCE | METRIC_DOMAIN | AUTHORITY | COMPARABILITY | FRESHNESS | WHAT_IT_PROVES | WHAT_IT_DOES_NOT_PROVE |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| E-G1-GEOM8 | P1/P2/P3 | `v044-p1-default-worker-geometry-8-20260822T035125Z` | RPS, CPU, RSS, latency | CURRENT | PASS (geom8 3-way) | 2026-08-22 | Coordinated parallelism default; P1 −8.8% RPS vs NGINX post-fix; P2 RPS win; P3 win preserved | P4 applicability; syscall/perf deep dive |
| E-P1-ATTR | P1 | `v044-p1-competitive-deficit-attr-20260821T211202Z` | PSS, threads, FD | HISTORICAL (pre-G1 numbers) | PASS | 2026-08-21 | Fat binary + anon runtime ≈9.5 MiB PSS gap vs NGINX; FD cache not primary | Current geom8 RPS; P4 |
| E-AB3 | P1 | `v044-p1-fixed-stage-parallelism-ab-20260822T024222Z` | Parallelism A/B | CURRENT (causal) | PASS | 2026-08-22 | Matched 8/8/8 +10.4% RPS; roots/handle registry below resolution | P4; post-G1 residual |
| E-P2-C2 | P2 | `v044-p2-static-64k-3way-20260821T212806Z` | RPS, CPU | SUPERSEDED by G1 session | PASS | 2026-08-21 | Direction: CPU win on static 64K | Current geom8 authority |
| E-P3-C1 | P3 | `v044-p3-static-1mib-3way-20260821T224438Z` | RPS, CPU, tail | SUPERSEDED by G1 session | PASS | 2026-08-21 | Large-static win possible | Current geom8 authority |
| E-ARCH-P4 | P1–P3 | `v044-architecture-fitness-review-before-p4.md` | Cost model F+k·bytes | CURRENT (analysis) | PASS | 2026-08-22 | Fixed cost ≈11 µs; payload amortization; P1 RPS paradox | P4 proxy path |
| E-P4-115113Z | P4 | `v044-p4-proxy-1k-3way-20260822T115113Z` | RPS, CPU, RSS, tail | CURRENT | PASS | 2026-08-22 | ~24% RPS deficit; +39% CPU vs NGINX; tail wins | Causal CPU classes; pool reuse; perf/flame |
| E-P4-INVALID | P4 | `v044-p4-proxy-1k-3way-20260822T112732Z` | — | INVALID_NON_AUTHORITATIVE | FAIL | — | Harness defect only | Anything competitive |
| E-P1-CAUSAL | P1 | `v044-p1-cpu-efficiency-causal.md` + evidence dir | strace, cgroup, flame | HISTORICAL (pre-G1 binary) | PASS (meter) | 2026-08-20 | Userspace static control plane vs NGINX; syscall geometry | Current geom8 seal; P4 |
| E-ROOT1-3 | P1 | repair-root1/2/3 governance | CPU repairs | CLOSED / FALSIFIED | PASS | 2026-08 | Root1/3 proven; Root2 falsified | P4 |
| E-CAP067-FD | P1 | `v044-p1-fd-reuse-repair.md` | FD geometry | CLOSED | PASS | 2026-08 | sendfile path sound | P4 (no Cap067) |
| E-CAP067-H1 | P1 | `v044-p1-cap067-miss-needhyper-hang.md` | miss/hang | CLOSED | PASS | 2026-08 | Alternative B terminals | P4 |
| E-P4-PATH | P4 | source code map (proxy wire) | path reconstruction | SOURCE_CONFIRMED | N/A | current tree | Tokio/Hyper proxy; no Cap067; WAF/access off on bench | Measured µs per stage |

```text
P4_PROFILING_ARTIFACTS = NOT_MEASURED
  # no perf record / flamegraph / strace on authoritative P4 binary in lab or Netcup evidence index
INSTRUCTIONS_PER_REQ_CURRENT_GEOM8 = NOT_MEASURED
UPSTREAM_POOL_METRICS_P4 = NOT_MEASURED
```

---

## Authoritative profile truth (geom8 product authority)

| Profile | ExyonQ RPS | vs NGINX RPS | CPU/req vs NGINX | RSS ExyonQ | COMPETITIVE_WIN |
| --- | ---: | ---: | ---: | ---: | --- |
| P1 1 KiB | 154062 | −8.8% | **−19.3%** (win) | 13.86 MiB | NOT_PROVEN |
| P2 64 KiB | 104041 | **+15.3%** | **−16.0%** (win) | 21.64 MiB | NOT_PROVEN |
| P3 1 MiB | 8683 | +1.0% | **−0.4%** (win) | 71.95 MiB | **YES** |
| P4 proxy 1 KiB | 73097 | −24.2% | **+39.4%** (loss) | 33.18 MiB | NOT_PROVEN |

```text
PARALLELISM_UNDERUTIL = CLOSED_FIXED_AND_REBENCHMARKED
  # G1 product change; do not reopen 4/4/4 vs 8/8/8 without new contradicting evidence
```

---

## Cross-profile cost model

```text
TOTAL_REQUEST_COST =
  CLIENT_IO + REQUEST_PARSE + ROUTING + POLICY_SECURITY + MODULE_DISPATCH
  + FILE_OR_UPSTREAM_WORK + RESPONSE_BUILD + RESPONSE_TRANSFER
  + CONNECTION_LIFECYCLE + SCHEDULING_HANDOFF + SYNCHRONIZATION
  + ALLOCATION_COPY + OBSERVABILITY + KERNEL_COST
```

### Measured / bounded estimates (geom8 authority)

| Profile | core-ms/req ExyonQ | user µs/req | system µs/req | SYSCALLS/req | ALLOC/req | RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| P1 | 0.130 | NOT_MEASURED | NOT_MEASURED | ~6.2 (historical strace) | NOT_MEASURED | 13.86 |
| P2 | 0.197 | NOT_MEASURED | NOT_MEASURED | NOT_MEASURED | NOT_MEASURED | 21.64 |
| P3 | 1.146 | 7.58 (historical cgroup) | 150.0 (historical) | NOT_MEASURED | NOT_MEASURED | 71.95 |
| P4 | 0.207 | **21.90** | **18.68** | NOT_MEASURED | NOT_MEASURED | 33.18 |

Comparator P4 user µs/req: NGINX **6.78**, OLS **7.72** — ExyonQ user CPU **≈3.2× NGINX**.

Fixed-cost linear fit (P1+P3, docker-stats µs/req, architecture fitness review):

```text
ExyonQ F ≈ 11.4 µs + 0.086 µs/KiB
NGINX  F ≈ 13.6 µs + 0.090 µs/KiB
```

P4 total ≈207 µs/req — dominated by **upstream + dual HTTP stack + userspace proxy**, not static sendfile F.

| Metric | P1 | P2 | P3 | P4 |
| --- | --- | --- | --- | --- |
| CYCLES/req | NOT_MEASURED | NOT_MEASURED | NOT_MEASURED | NOT_MEASURED |
| INSTRUCTIONS/req | NOT_MEASURED | NOT_MEASURED | NOT_MEASURED | NOT_MEASURED |
| FUTEX/req | NOT_MEASURED | NOT_MEASURED | NOT_MEASURED | NOT_MEASURED |
| CONTEXT_SWITCHES/req | NOT_MEASURED | NOT_MEASURED | NOT_MEASURED | NOT_MEASURED |
| COPIED_BYTES/req | ~1 KiB (payload) | ~64 KiB | ~1 MiB (sendfile) | ~1 KiB (+ framing) inferred |
| PSS | ~20.2 MiB (attr session) | NOT_MEASURED | NOT_MEASURED | NOT_MEASURED |

---

## Core questions (evidence classification)

```text
IS_THERE_A_SHARED_CPU_ROOT_ACROSS_P1_P2_P4 = BOUNDED
  # Shared userspace/runtime tax class exists, but P1/P2 WIN CPU vs NGINX on static;
  # P4 LOSES CPU badly (+39%). Not one measurable shared CPU deficit.

IS_THERE_A_SHARED_MEMORY_ROOT_ACROSS_P1_P4 = STRONGLY_SUPPORTED
  # Fat monolith binary + runtime anon; PSS gap ~5–10 MiB vs NGINX scales with profile state.

IS_PROXY_PATH_ARCHITECTURE_STRUCTURALLY_EXPENSIVE = STRONGLY_SUPPORTED
  # P4: +39% CPU vs NGINX, +57% vs OLS; user µs/req 3.2× NGINX; −24% RPS.

IS_RUNTIME_HYBRID_TOKIO_EPOLL_CAUSING_SYSTEMIC_COST = BOUNDED
  # Material on P1 first-request / static seam; REJECTED as P4 primary (P4 is pure Tokio).

IS_CONNECTION_OWNERSHIP_OR_HANDOFF_COST_SYSTEMIC = BOUNDED
  # P1: Tokio→Cap067 divert bounded to first request on static KA.
  # P4: stream boxing + dual HTTP stacks STRONGLY_SUPPORTED as path cost (not yet µs-quantified).

IS_UPSTREAM_POOLING_OR_PROXY_STATE_MACHINE_INEFFICIENT = INSUFFICIENT_EVIDENCE
  # UPSTREAM_CONNECTION_REUSE_METRICS = NOT_MEASURED; keepalive qualitatively enabled.

IS_REQUEST_PATH_LAYERING_TOO_DEEP = STRONGLY_SUPPORTED
  # P4: ~21 paid stages Tokio→route→proxy wire→Hyper→upstream→chunked client (code map).
  # P1 Cap067 hit: deeper than nginx loop but CPU competitive post-G1.

IS_OBSERVABILITY_SECURITY_CONTROL_PLANE_RESIDENCY_TOO_EXPENSIVE = BOUNDED
  # P1 AB3: WAF/roots/handle registry BELOW_RESOLUTION with WAF off.
  # P4 bench: WAF/access off; still pays generation pin, cluster RwLock, header materialization.

IS_PROCESS_IMAGE_OR_ALLOCATOR_FOOTPRINT_SYSTEMIC = STRONGLY_SUPPORTED
  # Private_Clean ≈ binary 9–10 MiB; P1 13.9 vs NGINX 8.4 MiB RSS; P4 33.2 vs 9.95 MiB.

IS_SHARED_STATE_OR_SYNCHRONIZATION_SYSTEMIC = BOUNDED
  # P1: roots RwLock / handle registry REJECTED_NOT_MATERIAL (AB3 perf).
  # P4: cluster RwLock on path; contention NOT_MEASURED.

IS_KERNEL_SYSCALL_GEOMETRY_SYSTEMIC = BOUNDED
  # P1 historical: openat2+sendfile+stat ≈6.2/req; not remeasured on geom8.
  # P4: NOT_MEASURED.

IS_MEMORY_COPY_BUFFER_GEOMETRY_SYSTEMIC = BOUNDED
  # P3: sendfile zero-copy wins; P4: chunked TE + Hyper frames (inferred); P1: negligible at 1 KiB.

IS_CURRENT_ARCHITECTURE_CAPABLE_OF_PARETO_COMPETITIVENESS = BOUNDED
  # PROVEN for large static (P3); NOT_PROVEN for small static or proxy at migration bar.
```

---

## P3 differential (control profile)

```text
WHAT_COSTS_ARE_AMORTIZED_IN_P3_BUT_DOMINANT_IN_P1_P2_P4 =
  Fixed per-request framework cost (~11 µs class) becomes negligible vs ~100+ µs
  kernel sendfile transfer at 1 MiB. Wall time → KERNEL_TX_COST dominated.

WHAT_PATH_COMPONENTS_EXIST_IN_P4_BUT_NOT_P3 =
  Upstream TCP + Hyper client pool; dual HTTP parse (client + upstream);
  proxy route/cluster resolution; chunked TE response encoding; Connection: close
  client policy (Cap034); no Cap067 sendfile to client.

WHAT_PATH_COMPONENTS_EXIST_IN_P1_P2_P3_BUT_NOT_P4 =
  Cap067 epoll sendfile FSM; static wire fast path; FD cache; site_static roots;
  epoll_worker keepalive pump (post-first-request on static).

WHAT_COMMON_COMPONENTS_EXIST_ACROSS_P1_P2_P4 =
  Tokio accept/read (always on P4; first req on static); fat process image;
  generation/reload pin checks; route index; bench RAW (WAF/access/modules off).

WHY_DOES_EXYONQ_WIN_P3_BUT_LOSE_P4_CPU_PER_REQ =
  P3 executes competitive Cap067 kernel sendfile with payload-dominated cost matching
  rivals. P4 executes full userspace Tokio/Hyper proxy — user CPU 21.9 µs vs NGINX 6.8 µs.

WHY_DOES_EXYONQ_P4_HAVE_GOOD_P95_P99_BUT_LOW_RPS =
  Lower RPS CV (0.026 vs 0.070 NGINX) and conservative pacing/chunked write pattern
  smooth tails; higher per-request CPU (~207 µs) caps throughput (~73k vs ~96–97k).

WHY_IS_EXYONQ_P4_RSS_33.18_MIB =
  Base process ~14 MiB + Hyper idle pools + proxy cluster/runtime state + upstream
  connection bookkeeping vs NGINX lean ~10 MiB multi-process (~3.3× gap).
```

---

## P4 request path reconstruction (summary)

Authoritative path: **proxy wire fast path** — Tokio only; Cap067 **not** entered for `GET /api/`.

| Stage | Class | Runtime | Ownership / sync | Notes |
| --- | --- | --- | --- | --- |
| TCP accept | ESSENTIAL | Tokio | stream → task | |
| Header read | ESSENTIAL | Tokio | Bytes head | |
| Wire plan → Proxy | ESSENTIAL | Tokio | | |
| Route match | ESSENTIAL | Tokio | Arc snapshot read | |
| WAF wire | SECURITY_REQUIRED | Tokio | early-out bench | |
| XFF string | ESSENTIAL | Tokio | alloc/request | |
| Stream box dyn | CONTRACT_REQUIRED | Tokio | Box<dyn> | |
| Cluster resolve | ESSENTIAL | Tokio | **RwLock read** | |
| Hyper upstream GET | ESSENTIAL | Tokio+Hyper | pool | |
| Response head encode | ESSENTIAL | Tokio | Vec alloc | |
| Chunked TE write | ESSENTIAL | Tokio | per-frame write | |
| Access terminal | OPTIONAL_BUT_PAID | Tokio | no-op bench | timing still entered |

Fallback Hyper full stack exists but is **not** nominal P4 hot path.

---

## Memory attribution

### P1 memory gap roots

```text
P1_MEMORY_GAP_ROOTS =
  PRIMARY (~5–6 MiB RSS vs NGINX geom8): fat binary Private_Clean + anon runtime
  SECONDARY: shared libs; heap label small in attr session
  FD_CACHE: REJECTED_NOT_MATERIAL
SHARED: same monolith image as P4 base
```

### P4 memory gap roots

```text
P4_MEMORY_GAP_ROOTS =
  PRIMARY (~23 MiB vs NGINX 9.95): base image (~14 MiB) + proxy/Hyper pool state (~8–12 MiB class, NOT_MEASURED breakdown)
  SECONDARY: upstream idle connections (qualitative keepalive)
UNEXPLAINED: smaps/PSS breakdown NOT_MEASURED on P4 authority run
```

### Shared memory roots P1–P4

```text
SHARED_MEMORY_ROOTS_P1_P4 =
  ROOT_MEMORY_FOOTPRINT — STRONGLY_SUPPORTED — ~5–10 MiB structural vs NGINX
  P4 adds incremental proxy runtime state on top of shared base
```

---

## CPU cost classes (P4 — best available evidence)

```text
TOP_EXYONQ_CPU_COST_CLASSES (P4, inferred from cgroup user/system split + code map):
  1. Userspace HTTP proxy stack (Tokio poll + Hyper client/server framing) — STRONGLY_SUPPORTED
  2. Upstream I/O scheduling (read response, write request) — STRONGLY_SUPPORTED
  3. Response chunked encoding to client — BOUNDED (inferred)
  4. Kernel socket I/O (system µs comparable to NGINX) — OBSERVED (18.7 vs 21.7 µs)

TOP_NGINX_CPU_COST_CLASSES (P4, OBSERVED meter only):
  1. Lean event-loop proxy (low user µs)
  2. Kernel upstream/client socket I/O (system µs dominant)

TOP_OLS_CPU_COST_CLASSES (P4, OBSERVED meter only):
  1. Lowest total core-ms/req (0.132)
  2. Balanced user/system split

EXCESS_EXYONQ_CPU_COST_CLASSES =
  Userspace proxy framework: ~+15 µs user/req vs NGINX (~+220% user class; meter-derived)
  NOT YET decomposed into perf symbols — INSUFFICIENT_EVIDENCE for sub-class %
```

---

## Ranked root ledger

### ROOT_1 — PROXY_USERSPACE_STACK_TAX

```text
ROOT_ID = ROOT_1_PROXY_USERSPACE_STACK_TAX
ROOT_NAME = Tokio/Hyper layered proxy path vs lean C reverse-proxy event loop
AFFECTED_PROFILES = P4 (primary); not P1/P2/P3 static sendfile paths
EVIDENCE_STRENGTH = STRONGLY_SUPPORTED (meter); causal decomposition INSUFFICIENT_EVIDENCE
MEASURED_COST = +39.4% core-ms/req vs NGINX; −24.2% RPS; user µs/req ≈3.2× NGINX
RECOVERABLE_COST = NOT_BOUNDED (no perf/flame on geom8 P4 binary)
EXPECTED_RPS_GAIN = NOT_BOUNDED
EXPECTED_CPU_GAIN = NOT_BOUNDED
EXPECTED_MEMORY_GAIN = NONE (this root is CPU/path)
EXPECTED_P50_EFFECT = unknown
EXPECTED_P95_EFFECT = risk of regression (ExyonQ currently wins P95/P99)
EXPECTED_P99_EFFECT = risk of regression
CROSS_PROFILE_APPLICABILITY = P4-only
IMPLEMENTATION_COMPLEXITY = HIGH (proxy wire redesign / slimmer data plane)
CORRECTNESS_RISK = HIGH
SECURITY_RISK = MEDIUM (must preserve WAF/proxy contracts)
PORTABILITY_RISK = MEDIUM
MAINTENANCE_RISK = HIGH
ARCHITECTURE_SCOPE = BOUNDED_CROSS_CUTTING (proxy subsystem)
MIGRATION_VALUE_EFFECT = MATERIAL if recoverable fraction is large
RECOMMENDATION = MEASURE_MORE then TARGETED_PRODUCT_WIP
```

### ROOT_2 — MEMORY_FOOTPRINT_MONOLITH

```text
ROOT_ID = ROOT_2_MEMORY_FOOTPRINT_MONOLITH
ROOT_NAME = Fat Rust monolith binary + runtime anon vs lean multi-process rivals
AFFECTED_PROFILES = P1, P2, P4 (P3 near-parity RSS vs NGINX at large static)
EVIDENCE_STRENGTH = STRONGLY_SUPPORTED
MEASURED_COST = P1 +5.4 MiB RSS; P4 +23.2 MiB RSS vs NGINX
RECOVERABLE_COST = PARTIAL (LTO/strip/deps) — unlikely to match NGINX 8–10 MiB
EXPECTED_RPS_GAIN = LOW (not primary RPS driver on P1)
EXPECTED_CPU_GAIN = NONE
EXPECTED_MEMORY_GAIN = PARTIAL (~2–5 MiB class hypothetical; NOT_BOUNDED)
EXPECTED_TAIL_EFFECT = NEUTRAL
CROSS_PROFILE_APPLICABILITY = P1/P2/P4 shared
IMPLEMENTATION_COMPLEXITY = MEDIUM–HIGH
CORRECTNESS_RISK = LOW
SECURITY_RISK = LOW
ARCHITECTURE_SCOPE = BOUNDED_CROSS_CUTTING
MIGRATION_VALUE_EFFECT = PARTIAL (memory alone weak migration case per P1 attribution)
RECOMMENDATION = DEFER vs proxy CPU root; no standalone product WIP justified
```

### ROOT_3 — SMALL_PAYLOAD_DELIVERY_RESIDUAL

```text
ROOT_ID = ROOT_3_SMALL_PAYLOAD_DELIVERY_RESIDUAL
ROOT_NAME = Post-G1 residual RPS/latency gap on 1 KiB static despite winning CPU vs NGINX
AFFECTED_PROFILES = P1 (primary); P2 less severe (RPS win); not P3/P4 same mechanism
EVIDENCE_STRENGTH = STRONGLY_SUPPORTED (symptom); causal decomposition BOUNDED
MEASURED_COST = P1 −8.8% RPS vs NGINX; P50/P95/P99 losses vs NGINX
RECOVERABLE_COST = NOT_BOUNDED (stage depth AB3 below resolution; parallelism closed)
EXPECTED_RPS_GAIN = NOT_BOUNDED
EXPECTED_CPU_GAIN = unknown
EXPECTED_MEMORY_GAIN = NONE
CROSS_PROFILE_APPLICABILITY = P1/P2 small-static class only
IMPLEMENTATION_COMPLEXITY = MEDIUM (Cap067 stage thinning / first-request seam)
CORRECTNESS_RISK = MEDIUM
ARCHITECTURE_SCOPE = LOCAL (static wire)
MIGRATION_VALUE_EFFECT = PARTIAL
RECOMMENDATION = MEASURE_MORE (perf-stat/instructions on geom8 P1) before product WIP
```

### Rejected / closed roots (do not reopen without new evidence)

| Root | Status | Notes |
| --- | --- | --- |
| PARALLELISM_UNDERUTIL | CLOSED_FIXED (G1) | Residual P1 gap is not this root |
| ROOTS_RWLOCK | REJECTED_NOT_MATERIAL | AB3 perf sample |
| HANDLE_REGISTRY | REJECTED_NOT_MATERIAL | AB3 + Root2 falsified |
| FD_CACHE RSS | REJECTED_NOT_MATERIAL | P1 attribution |
| RUST_LANGUAGE | REJECTED | P3 win |
| GLOBAL_ARCHITECTURE_BROKEN | REJECTED | P3 win |

---

## Architecture options re-evaluation

| Option | Profiles helped | RPS effect | CPU effect | Memory | Tail | Evidence | Recommendation |
| --- | --- | --- | --- | --- | --- | --- | --- |
| A KEEP_CURRENT | P3 | preserve win | preserve | unchanged | preserve | PROVEN P3 | insufficient alone |
| B TARGETED_EVOLUTION | P1/P4 | bounded upside | ↓ fixed/proxy | maybe ↓ | ↓ P1? | STRONGLY_SUPPORTED | **YES (direction)** |
| C WORKER_LOCAL_OWNERSHIP | P1/P4 | BOUNDED | BOUNDED | neutral | unknown | BOUNDED | subset of B |
| D THREAD_PER_CORE | P1? | NOT_BOUNDED | unknown | unknown | unknown | INSUFFICIENT_EVIDENCE | RESEARCH_ONLY |
| E SEPARATE_DATA_PLANE | P4/P1 | NOT_BOUNDED | potentially large | unknown | unknown | HYPOTHESIS | RESEARCH_ONLY |
| F IO_URING | P1/P3 | NOT_BOUNDED | unknown | unknown | unknown | NOT_JUSTIFIED_YET | RESEARCH_ONLY |
| G MAJOR_DATA_PLANE_REDESIGN | P4 | NOT_BOUNDED | potentially large | unknown | P3 risk | HYPOTHESIS | NOT_AUTHORIZED |
| H MAJOR_REWRITE | all | NOT_BOUNDED | unknown | unknown | P3 risk | DISPROVEN need | **NO** |

```text
TARGETED_EVOLUTION_SUFFICIENCY = NOT_PROVEN
  # P3 proves ceiling exists; P4 proxy gap may require substantial proxy-subsystem work
THREAD_PER_CORE_RECOMMENDATION = RESEARCH_ONLY
IO_URING_RECOMMENDATION = RESEARCH_ONLY
SEPARATE_DATA_PLANE_RECOMMENDATION = RESEARCH_ONLY
```

---

## Rewrite bar check

```text
MAJOR_REARCHITECTURE_JUSTIFIED = NO
  # fails: P3 win; bounded roots identified; no proof local fixes cannot close material gaps
FULL_REWRITE_JUSTIFIED = NO
  # fails: P3 counter-evidence; C/C++ language not causal
```

---

## Terminal proposals

```text
NEXT_PRODUCT_WIP_PROPOSAL = NONE
  # superseded 2026-08-26: PXDP-B design complete
  # bounded by execution-tax decomposition CASE PX-B — owner authorization required

NEXT_MEASUREMENT_WIP_PROPOSAL = NONE
  # completed: V044_P4_PROXY_EXECUTION_TAX_DECOMPOSITION — CASE PX-B

SUCCESSOR_DOC = docs/governance/v044-p4-proxy-execution-tax-decomposition.md
EXPECTED_USER_VALUE = BOUNDED partial P4 gap closure (12–22% RPS non-overlapping range)
EXPECTED_MIGRATION_VALUE_EFFECT = NOT_PROVEN
```

```text
P5_STARTED = NO
P5_TO_P14_FROZEN = YES
DOCS_RECONCILED = YES
NOTION_UPDATE_REQUIRED = YES
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
PUSH = NO
TAG = NO
RELEASE = NO
```

---

## Profile summaries

```text
P1_ROOT_SUMMARY =
  Post-G1: wins CPU vs NGINX (−19% core-ms) but loses RPS (−8.8%) and latency vs NGINX.
  Primary open classes: ROOT_3 delivery residual + ROOT_2 memory. Parallelism closed.

P2_ROOT_SUMMARY =
  Wins RPS (+15% vs NGINX) and CPU; loses P95/P99 vs OLS and RSS vs NGINX.
  Payload amortization helps; not migration-grade (COMPETITIVE_WIN NOT_PROVEN).

P3_CONTROL_PROFILE_SUMMARY =
  Competitive win preserved (CPU + tail + near-parity RPS). Proves architecture can win
  when kernel sendfile dominates. Any systemic explanation must preserve this.

P4_ROOT_SUMMARY =
  Material −24% RPS vs both rivals despite P95/P99 wins. CPU/request +39% vs NGINX
  driven by userspace proxy stack (user µs ≈3.2× NGINX). ROOT_1 dominant; needs profiling.
```
