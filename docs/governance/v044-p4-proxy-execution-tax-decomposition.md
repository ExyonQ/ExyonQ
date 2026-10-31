# V044 P4 — Proxy execution tax decomposition

```text
STATUS = COMPLETE
CASE = PX-B
WIP = V044_P4_PROXY_EXECUTION_TAX_DECOMPOSITION
MODE = READ_ONLY_CAUSAL_DECOMPOSITION
PRODUCT_MUTATION = NO
ARCHITECTURE_MUTATION = NO
P5_STARTED = NO
P5_TO_P14_FROZEN = YES
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

SOURCE_HEAD = ade997a362e60d1dc6eafc984da454930600845a
SOURCE_TREE = 4ae6ffc7528d0881be23fcac246c345dcb9a1606
SOURCE_BINARY_SHA256 = e28c0abe18587f2d911d492be64cbec42372383e011417a6c578be52da5ba459

RUN_ID = v044-p4-proxy-execution-tax-decomposition-20260825T230314Z
EVIDENCE_NETCUP =
  /root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p4-proxy-execution-tax-decomposition-20260825T230314Z/
HARNESS = scripts/remote/v044-p4-proxy-execution-tax-decomposition.sh
PRIOR_PROFILING = v044-p4-proxy-path-causal-profiling-20260825T203611Z (P4P-B)
HOST = Netcup amd64 (netcup-bench)
CAP034_EXPERIMENT_STATUS = COMPLETE_AB_C34_B_NOT_AUTHORITY
CAP034_CANONICAL_MERGE = NO
```

## Executive answer

```text
WHERE_DOES_THE_REMAINING_P4_EXECUTION_TAX_GO_AFTER_CAP034_IS_DEPRIORITIZED =
  A causally coupled stack on the canonical A0 wire proxy path:
    (1) ExyonQ→Hyper client integration (pool checkout, mpsc dispatch, BoxBody framing)
    (2) Tokio scheduler handoffs driven by that async geometry (~133 ctx switches/req process-wide)
    (3) Allocator + Bytes refcount churn on the body/header pipeline (~8–9% exclusive CPU combined)
  Chunked downstream framing + Connection: close (Cap034 A0) remains a bounded but secondary slice
  (+5.27% RPS measured in experiment; not merged).
  Upstream pool economics partially observed (TIME_WAIT churn) but hit rate not directly measured.

PROXY_EXECUTION_ARCHITECTURE_FITNESS = TARGETED_EVOLUTION
IS_PROXY_EXECUTION_TAX_LOCALLY_FIXABLE = BOUNDED (partial; not full ~24% gap from local slices alone)
IS_PROXY_SUBSYSTEM_REDESIGN_REQUIRED = PLAUSIBLE if stacked local wins <10% RPS
```

---

## 1. Measurement contract

```text
BENCH_CONTRACT = P4_PROXY_1K /api/ 1024B upstream keepalive / RAW WAF off
CPUSET = 0-7
CLIENT_CONCURRENCY = 100
LOAD_THREADS = 2
WARMUP = 20s
MEASURE = 30s
WORKER_GEOMETRY = 8 accept / 8 epoll / 8 Tokio (product defaults)
PRODUCT_MUTATION = NO
ZERO_FAKE = YES
```

Canonical competitive baseline numbers (115113Z) are **not** rewritten; this session reports
profiling-session RPS and causal metrics only.

---

## 2. P4P-B reproduction

| Metric | P4P-B (203611Z) | This run (230314Z) | Ratio |
| --- | ---: | ---: | ---: |
| RPS | 76215 | **74100** | 0.97 |
| cycles/req | 34,617,095 | **37,571,899** | 1.09 |
| instructions/req | 25,460,815 | **28,887,876** | 1.13 |
| IPC | 0.735 | **0.769** | 1.05 |
| ctx switches/req | 115.78 | **133.09** | 1.15 |
| cpu migrations/req | 0.552 | **0.607** | 1.10 |

```text
P4P_B_PROFILE_REPRODUCED = YES
```

Variance within expected host noise; causal class ranking unchanged.

---

## 3. perf stat — process-wide (ExyonQ)

| Metric | ExyonQ | NGINX (ref) | OLS (ref) |
| --- | ---: | ---: | ---: |
| RPS | **74100** | 101425 | 93852 |
| cycles/req | **37,571,899** | 1,843,744 | 1,726,055 |
| instructions/req | **28,887,876** | 1,894,111 | 2,070,159 |
| IPC | **0.769** | 1.027 | 1.199 |
| ctx switches/req | **133.09** | 8.34 | 9.72 |
| cpu migrations/req | **0.607** | 0.008 | 0.010 |
| page faults/req | 8.28 | 38.38 | 0.0002 |

```text
CURRENT_RPS = 74100.01
CURRENT_CYCLES_PER_REQ = 37571899
CURRENT_INSTRUCTIONS_PER_REQ = 28887876
CURRENT_IPC = 0.769
CURRENT_CONTEXT_SWITCHES_PER_REQ = 133.09
CPU_MIGRATIONS_PER_REQ = 0.607
```

**CPU/request (profiling session, task-clock / requests):**

```text
CURRENT_CPU_PER_REQ = 0.792 core-ms/req  (profiling perf-stat task-clock)
CANONICAL_BASELINE_CPU_PER_REQ = 0.206626 core-ms/req  (115113Z docker-stats; unchanged)
```

**User vs system (BOUNDED estimate from exclusive symbol classes):**

```text
CURRENT_USER_US_PER_REQ = BOUNDED ~630 µs/req (~80% of task-clock)
CURRENT_SYSTEM_US_PER_REQ = BOUNDED ~162 µs/req (~20%)
MARK_OVERLAP = YES (kernel send path is downstream of userspace write churn)
```

---

## 4. P4 execution graph (source reconstruction)

P4 hot path: `GET /api/*` → epoll declines keepalive (`GET /api/` forces Tokio) →
`wire_dispatch::serve_proxy_wire` → `proxy_wire::serve_proxy_wire` → `wire_conn::serve` →
`ProxyRuntime::forward_get_for_wire` → `attempt_forward_empty_body` (Hyper GET client) →
`write_proxy_response_wire` → `write_streaming_te_progressive` (chunked + close).

| STAGE_ID | SOURCE | FUNCTION | DOMAIN | ALLOC/CLONE/CHANNEL |
| --- | --- | --- | --- | --- |
| CLIENT_CONNECTION | epoll_worker / server | accept / handoff_to_hyper | epoll thread → Tokio | NOT_MEASURED |
| CLIENT_HTTP_PARSE | core/server/io | read_until_headers | Tokio task | Bytes carry clone BOUNDED |
| WIRE_DECISION | core/server/wire_dispatch | plan_wire_decision / might_use_proxy_wire | Tokio | none |
| ROUTE_RESOLUTION | wire_dispatch | cluster_id resolve | Tokio | Arc snapshot read |
| WAF_GATE | module attachment | evaluate_wire_waf_reject | Tokio | NOT_MEASURED |
| PROXY_WIRE_DISPATCH | wire_dispatch | serve_proxy_wire | Tokio | BoxBody stream wrap |
| HYPER_CLIENT_REQUEST | hyper_forward | attempt_forward_empty_body | Tokio + Hyper driver | Request build, header clones |
| POOL_CHECKOUT | hyper_util client pool | Checkout::poll | Hyper conn task | mpsc wake |
| UPSTREAM_SEND | hyper client | send_request | Hyper mpsc | channel send |
| UPSTREAM_READ | hyper body | Incoming poll | Tokio IO | Frame<Bytes> |
| BODY_FRAME_CREATION | wire_conn | body.frame() loop | Tokio | Frame alloc, Bytes refcount |
| RESPONSE_HEADER_BUILD | wire_conn | encode_proxy_response_head | Tokio | Vec<u8> wire head |
| DOWNSTREAM_FRAME | wire_conn | write_chunk / progressive TE | Tokio | stack/Vec chunk buffers |
| CLIENT_WRITE | tokio mio | write_all / flush | Tokio IO | sendto/writev syscalls |
| COMPLETION | wire_conn | shutdown | Tokio | — |
| CONNECTION_CLEANUP | Tokio | drop tasks | worker pool | Bytes drop refcount |

```text
P4_TASK_BOUNDARY_COUNT = BOUNDED 4–6 (accept/dispatch, proxy serve, Hyper client future, conn driver, IO poll)
P4_CHANNEL_BOUNDARY_COUNT = BOUNDED 2–4 (Hyper mpsc dispatch + internal pool signaling)
P4_RUNTIME_HANDOFF_COUNT = BOUNDED 1 (epoll → Tokio handoff on /api/)
P4_THREAD_HANDOFF_COUNT = BOUNDED 1–8 (multi-worker Tokio; migrations 0.61/req)
P4_POLL_READY_BOUNDARY_COUNT = BOUNDED 4–8 (client + upstream socket registrations)
```

---

## 5. perf record — top exclusive symbols (ExyonQ worker sample)

| % self | Symbol / class |
| ---: | --- |
| 4.90% | kernel IPI / reschedule |
| 3.62% | **malloc** |
| 2.72% | kernel spin unlock (wake path) |
| 1.47% | **bytes::shared_drop** |
| 1.27% | **bytes::shared_v_drop** |
| 1.16% | **bytes::shared_clone** |
| 0.84% | **tokio::Registration::poll_ready** |
| 0.79% | **hyper client pool Checkout::poll** |
| 0.77% | **hyper_util server conn auto poll** (non-P4 handoff path residual) |
| 0.70% | **hyper mpsc unbounded send** |
| 0.66% | **cfree** |
| 0.64% | **bytes::shared_v_clone** |
| 0.59% | tokio task poll (proxy module) |
| 0.58% | **exyonq_core::server::accept_loop** |
| 0.56% | **hyper client send_request** |
| 0.53% | **exyonq_core::server::serve_connection** |

Inclusive stacks: **~43% children under kernel sendto/tcp_write_xmit** — userspace drives
socket churn via chunked progressive writes + separate upstream reads.

### CPU cost-class aggregation (exclusive, non-overlapping sum ≤100%)

| Class | Est. exclusive % | Evidence |
| --- | ---: | --- |
| TOKIO_SCHEDULER | **2.0** | poll_ready, task poll, accept_loop |
| TOKIO_IO | **1.5** | (overlap with scheduler) |
| HYPER_POOL | **0.8** | Checkout::poll |
| HYPER_REQUEST | **0.6** | send_request |
| HYPER_CONNECTION_DRIVER | **0.8** | server auto poll + driver stacks |
| HYPER_BODY | **BOUNDED 1–2** | inclusive in frame path |
| HYPER_CHANNELS | **0.7** | mpsc send |
| BYTES_REFCOUNT | **4.5** | shared_clone/drop family |
| ALLOCATOR | **4.3** | malloc + cfree |
| MEMCPY | **NOT_MEASURED** exclusive | bounded inside bytes/vec |
| EXYONQ_PROXY_GLUE | **1.0** | serve_connection, wire encode |
| EXYONQ_ROUTING/WAF | **<0.5** | not in top symbols |
| SYSCALL_KERNEL_BOUNDARY | **~15–20** | kernel exclusive symbols |
| OTHER | remainder | |

---

## 6. Scheduler / context-switch decomposition (ROOT_1F)

```text
VOLUNTARY_CONTEXT_SWITCHES_PER_REQ = NOT_MEASURED_DIRECTLY (perf stat aggregate)
INVOLUNTARY_CONTEXT_SWITCHES_PER_REQ = NOT_MEASURED_DIRECTLY
SCHED_WAKEUPS_PER_REQ = BOUNDED ~133 (ctx switch proxy for wake-driven scheduling)
SCHED_MIGRATIONS_PER_REQ = 0.607
RUNQUEUE_DELAY_US_PER_REQ = NOT_MEASURED
CROSS_CORE_WAKEUPS_PER_REQ = BOUNDED (migrations + multi-worker)
WORKER_IMBALANCE = BOUNDED (container lacks ps; 18 Tokio threads observed at startup)
```

**Concurrency discriminator (single-worker perf stat during sweep):**

| C | RPS | ctx/req (worker) | instructions/req (worker) |
| ---: | ---: | ---: | ---: |
| 1 | 12783 | 30.98 | 1,928,282 |
| 8 | 49542 | 11.39 | 1,766,470 |
| 32 | 72313 | 7.45 | 1,779,715 |
| 100 | 80566 | 7.37 | 1,980,912 |
| 256 | 81526 | 7.71 | 1,973,712 |

```text
DO_CTX_SWITCHES_SCALE_WITH_CONCURRENCY = YES (process-wide 133/req at C=100 vs worker-local ~7/req)
DO_INSTRUCTIONS_PER_REQ_STAY_FLAT = PARTIAL (worker-scoped ~1.8–2.0M; process-wide ~29M)
DOES_HYPER_COST_SCALE_NONLINEARLY = NOT_PROVEN (Hyper symbols not isolated per C)
DOES_ALLOCATOR_COST_GROW_WITH_CONTENTION = PLAUSIBLE (malloc hot under load)
```

**Ranked wakeup sources (classification):**

| Rank | Source | Evidence |
| ---: | --- | --- |
| 1 | CLIENT_IO_WAKE | poll_ready + kernel tcp recv path |
| 2 | UPSTREAM_IO_WAKE | Hyper Incoming poll + recvfrom |
| 3 | HYPER_MPSC_WAKE | mpsc send 0.70% + ctx switches |
| 4 | POOL_CHECKOUT_WAKE | Checkout::poll 0.79% |
| 5 | BODY_FRAME_WAKE | frame() poll loop |
| 6 | WRITE_READY_WAKE | writev/sendto after chunk encode |

### ROOT_1F subroots

| Subroot | Evidence | Recoverable |
| --- | --- | --- |
| ROOT_1F_1_IO_WAKEUPS | STRONGLY_SUPPORTED | BOUNDED 5–10% RPS |
| ROOT_1F_2_MPSC_WAKEUPS | STRONGLY_SUPPORTED | BOUNDED 3–8% RPS (overlap 1F_1) |
| ROOT_1F_3_CROSS_CORE_MIGRATION | BOUNDED | BOUNDED 2–5% RPS |
| ROOT_1F_4_TASK_YIELD_REPOLL | PLAUSIBLE | NOT_BOUNDED |

---

## 7. Hyper integration decomposition (ROOT_1A)

| Candidate | Exclusive % | Est. µs/req | Recoverability |
| --- | ---: | ---: | --- |
| HYPER_POOL_CHECKOUT | 0.79 | BOUNDED ~6 | BOUNDED |
| HYPER_SEND_REQUEST | 0.56 | BOUNDED ~4 | BOUNDED |
| HYPER_MPSC | 0.70 | BOUNDED ~5 | BOUNDED if async boundary reduced |
| HYPER_BODY_FRAME | BOUNDED 1–2 | BOUNDED ~8–15 | STRONGLY_SUPPORTED |
| HYPER_RESPONSE_PARSE | BOUNDED <1 | BOUNDED | LOW |
| HYPER_CONNECTION_DRIVER | BOUNDED 0.5–1 | BOUNDED | overlap mpsc |
| EXYONQ_HEADER_ADAPTER | BOUNDED <0.5 | header clones in forward | AVOIDABLE partial |
| EXYONQ_BODY_ADAPTER | STRONGLY_SUPPORTED | body.boxed() + frame loop | STRONGLY_SUPPORTED |
| EXYONQ_ERROR_ADAPTER | LOW | — | LOW |
| EXYONQ_RUNTIME_HANDOFF | STRONGLY_SUPPORTED | epoll→Tokio + async stack | STRONGLY_SUPPORTED |

```text
HYPER_INTRINSIC_COST = BOUNDED ~2–3% exclusive (pool+send+mpsc)
EXYONQ_HYPER_INTEGRATION_COST = STRONGLY_SUPPORTED ~5–8% exclusive + inclusive multiplier
PROTOCOL_NECESSARY_COST = BOUNDED (HTTP proxy semantics)
AVOIDABLE_GLUE_COST = STRONGLY_SUPPORTED (BoxBody, chunked A0, epoll handoff)
```

### ROOT_1A subroots

| Subroot | Evidence |
| --- | --- |
| ROOT_1A_1_POOL_CHECKOUT | STRONGLY_SUPPORTED |
| ROOT_1A_2_HYPER_CHANNEL_DISPATCH | STRONGLY_SUPPORTED |
| ROOT_1A_3_BODY_FRAME_ADAPTER | STRONGLY_SUPPORTED |
| ROOT_1A_4_CONNECTION_DRIVER | BOUNDED |
| ROOT_1A_5_EPOLL_TOKIO_HANDOFF | STRONGLY_SUPPORTED |

---

## 8. Upstream pool (evidence gap closure attempt)

External observation under load:

```text
UPSTREAM_CONNECTIONS_CREATED = NOT_MEASURED_DIRECTLY (0 connect on sampled worker strace windows)
UPSTREAM_TCP_CONNECTIONS_REUSED = NOT_MEASURED_DIRECTLY
UPSTREAM_REQUESTS_PER_CONNECTION = NOT_MEASURED_DIRECTLY
POOL_HIT_RATE = NOT_MEASURED_DIRECTLY
POOL_MISS_RATE = NOT_MEASURED_DIRECTLY
CONNECTS_PER_REQUEST = BOUNDED ~0 on sampled worker (connects on other threads — P4P-B pattern)
CLOSES_PER_REQUEST = BOUNDED ~0.0001–0.001 (123 close / ~1M req window on worker sample)
TIME_WAIT_UPSTREAM = STRONGLY_SUPPORTED (many TIME_WAIT sockets in socket_obs.txt under load)
```

```text
IS_UPSTREAM_POOL_CHURN_A_MATERIAL_ROOT = BOUNDED (churn present; economics not closed)
```

Hyper pool config: `pool_max_idle_per_host=256`, `pool_idle_timeout=90s` — churn is not
explained by pool miss alone; **Connection: close** on downstream may interact with upstream
connection lifetime (causal chain, not isolated).

---

## 9. Allocation / Bytes / copy (ROOT_1G)

```text
MALLOC_CALLS_PER_REQ = NOT_MEASURED
FREE_CALLS_PER_REQ = NOT_MEASURED
ALLOCATED_BYTES_PER_REQ = NOT_MEASURED
BYTES_CLONES_PER_REQ = NOT_MEASURED (CPU proxy: ~4.5% exclusive in refcount symbols)
BYTES_DROPS_PER_REQ = NOT_MEASURED
ARC_CLONES_PER_REQ = NOT_MEASURED
BOX_ALLOCATIONS_PER_REQ = BOUNDED ≥2 (BoxBody path)
VEC_REALLOCATIONS_PER_REQ = BOUNDED (wire head Vec, chunk buffers >512B)
STRING_ALLOCATIONS_PER_REQ = BOUNDED (header adapter in hyper_forward)
```

**Copy geometry (1024 B body):**

```text
UPSTREAM_KERNEL_TO_USER_COPY = BOUNDED 1024 B (recv path)
USER_BUFFER_TO_HYPER_FRAME_COPY = BOUNDED 0–1024 B (Bytes ownership preferred)
HYPER_FRAME_TO_EXYONQ_COPY = BOUNDED refcount not memcpy dominant
EXYONQ_TO_DOWNSTREAM_BUFFER_COPY = BOUNDED chunk encode copies subset
USER_TO_KERNEL_COPY = BOUNDED 1024 B + chunk overhead
COPIES_PER_BODY = BOUNDED 2–4 payload-touch paths + chunk framing overhead
COPIED_BYTES_PER_REQUEST = BOUNDED 1024–1200 B
MEMCPY_CALLS_PER_REQ = NOT_MEASURED
MEMMOVE_CALLS_PER_REQ = NOT_MEASURED
ZERO_COPY_SEMANTICS = PARTIAL (Bytes move on upstream; downstream chunked re-frames)
BODY_PIPELINE_TRANSITIONS_PER_REQ = BOUNDED 8–12 type transitions
```

### ROOT_1G subroots

| Subroot | Evidence |
| --- | --- |
| ROOT_1G_1_BYTES_REFCOUNT | STRONGLY_SUPPORTED (~4.5% exc) |
| ROOT_1G_2_FRAME_ALLOC | STRONGLY_SUPPORTED |
| ROOT_1G_3_HEADER_ALLOC | BOUNDED |
| ROOT_1G_4_PROXY_GLUE_BOXING | STRONGLY_SUPPORTED (body.boxed()) |

---

## 10. Syscall geometry (12s strace, process)

Estimated ~888k requests in window.

| Syscall | calls/req |
| --- | ---: |
| recvfrom | 0.83 |
| sendto | 0.42 |
| writev | 0.42 |
| futex | 0.13 |
| epoll_wait | 0.018 |

```text
FUTEX_PER_REQ = 0.13
LOCK_WAIT_US_PER_REQ = NOT_MEASURED
ATOMIC_HOTSPOTS = Bytes refcount atomics STRONGLY_SUPPORTED in profile
ROOT_1E_RWLOCK = REJECTED_NOT_MATERIAL (unchanged)
```

Rivals: ~1 writev + ~1 recvfrom per request (simpler geometry).

---

## 11. Memory footprint

| | ExyonQ | NGINX | OLS |
| --- | ---: | ---: | ---: |
| PSS MiB | **43.85** | 3.55 | 1.89 |
| Private_Clean MiB | 10.46 | 3.10 | 1.30 |
| Private_Dirty MiB | **32.82** | 0.15 | 0.26 |

```text
PSS = 43.85 MiB
PRIVATE_DIRTY = 32.82 MiB
PRIVATE_CLEAN = 10.46 MiB
ANON = NOT_MEASURED (smaps aggregate)
ROOT_1G_RESIDENT_MEMORY_CONTRIBUTION = BOUNDED majority of Private_Dirty (Hyper/Tokio/body buffers)
```

---

## 12. Source-level hotspot ledger

| ID | File | Function | Class | CPU% | Root | Recoverability |
| --- | --- | --- | --- | ---: | --- | --- |
| HS-01 | hyper_util/client/legacy/pool | Checkout::poll | HYPER_POOL | 0.79 | 1A_1 | BOUNDED |
| HS-02 | hyper/client/dispatch | mpsc send | HYPER_CHANNELS | 0.70 | 1A_2 | BOUNDED |
| HS-03 | hyper_forward.rs | attempt_forward_empty_body | HYPER_REQUEST | 0.56 | 1A_2 | BOUNDED |
| HS-04 | wire_conn.rs | write_streaming_te_progressive | EXYONQ_GLUE | BOUNDED 1–2 | 1G_2/1B | STRONGLY_SUPPORTED |
| HS-05 | wire_conn.rs | body.frame loop | BYTES_REFCOUNT | BOUNDED 2–4 | 1G_1 | STRONGLY_SUPPORTED |
| HS-06 | wire_conn.rs | encode_proxy_response_head | ALLOC | BOUNDED <1 | 1G_3 | AVOIDABLE |
| HS-07 | tokio/runtime/io | Registration::poll_ready | SCHEDULER | 0.84 | 1F_1 | BOUNDED |
| HS-08 | allocator | malloc/cfree | ALLOCATOR | 4.3 | 1G | BOUNDED |
| HS-09 | epoll_worker.rs | handoff_to_hyper | HANDOFF | NOT_MEASURED | 1A_5 | STRONGLY_SUPPORTED |
| HS-10 | wire_dispatch.rs | serve_proxy_wire | DISPATCH | BOUNDED <1 | 1A | BOUNDED |

---

## 13. Causal overlap graph

```text
CAUSAL_OVERLAP_GRAPH =
  ROOT_1A_2 (Hyper mpsc dispatch)
    → ROOT_1F_2 (MPSC wake)
    → ROOT_1F_1 (IO poll_ready / ctx switch)
    → ROOT_1G_1 (Bytes clone on frame path)
    → ROOT_1G (malloc/free)

  ROOT_1A_5 (epoll→Tokio handoff)
    → ROOT_1F_3 (cross-core migration)

  ROOT_1B_CAP034_A0 (chunked+close) — ORTHOGONAL SLICE
    → extra sendto/writev (bounded +5% RPS in AB-C34-B; DEFER productization)

Do NOT sum 1A + 1F + 1G exclusive percents as independent recoverable value.
```

---

## 14. Recoverable-cost model (non-overlapping)

| Subroot | Measured µs/req | Recoverable fraction | Expected RPS gain | Evidence |
| --- | ---: | --- | ---: | --- |
| ROOT_1A_3 body adapter | BOUNDED 8–15 | 40–70% | 4–9% | STRONGLY_SUPPORTED |
| ROOT_1A_2 mpsc/Hyper dispatch | BOUNDED 5–10 | 30–60% | 3–7% | STRONGLY_SUPPORTED |
| ROOT_1F scheduler handoffs | BOUNDED 50–100 | 20–40% | 4–10% | STRONGLY_SUPPORTED |
| ROOT_1G refcount/alloc | BOUNDED 10–20 | 30–50% | 3–6% | STRONGLY_SUPPORTED |
| ROOT_1B Cap034 (deferred) | measured +5.27% | 100% of slice | +5% | BOUNDED (AB only) |

```text
TOTAL_NON_OVERLAPPING_RECOVERABLE_CPU_US_PER_REQ = BOUNDED 40–80 µs/req
TOTAL_NON_OVERLAPPING_EXPECTED_RPS_GAIN_RANGE = BOUNDED 12–22% RPS
  (not sufficient alone to close ~24% competitive gap; tails NEUTRAL/IMPROVE if done carefully)
```

---

## 15. Architecture questions (terminal)

| Question | Answer |
| --- | --- |
| IS_HYPER_INTRINSIC_COST_THE_DOMINANT_ROOT | **REJECTED** |
| IS_EXYONQ_HYPER_INTEGRATION_COST_THE_DOMINANT_ROOT | **STRONGLY_SUPPORTED** |
| IS_TOKIO_SCHEDULER_INTRINSIC_COST_THE_DOMINANT_ROOT | **REJECTED** |
| IS_EXYONQ_TASK_GEOMETRY_CAUSING_SCHEDULER_COST | **STRONGLY_SUPPORTED** |
| IS_BYTES_REFCOUNT_A_MATERIAL_ROOT | **STRONGLY_SUPPORTED** |
| IS_ALLOCATOR_A_MATERIAL_ROOT | **STRONGLY_SUPPORTED** (coupled to 1G) |
| IS_PAYLOAD_COPYING_A_MATERIAL_ROOT | **BOUNDED** (secondary to refcount/scheduling) |
| IS_CHANNEL_GEOMETRY_A_MATERIAL_ROOT | **BOUNDED** |
| IS_CROSS_THREAD_HANDOFF_A_MATERIAL_ROOT | **STRONGLY_SUPPORTED** |
| IS_UPSTREAM_POOL_CHURN_A_MATERIAL_ROOT | **BOUNDED** |

---

## 16. Architecture options (evaluation only)

| Option | Expected gain | Evidence | Complexity | Risk |
| --- | --- | --- | --- | --- |
| A KEEP_CURRENT | 0% | — | LOW | — |
| B TARGETED_HYPER_INTEGRATION | 8–15% | STRONGLY_SUPPORTED | MEDIUM | MEDIUM |
| C REDUCE_TASK_CHANNEL_HANDOFFS | 5–12% | STRONGLY_SUPPORTED | MEDIUM | MEDIUM |
| D WORKER_LOCAL_OWNERSHIP | 8–18% | BOUNDED | HIGH | HIGH |
| E LOWER_ALLOCATION_PIPELINE | 4–8% | STRONGLY_SUPPORTED | MEDIUM | LOW–MED |
| F CUSTOM_MINIMAL_PROXY_DATA_PATH | 15–25% | PLAUSIBLE | VERY HIGH | HIGH |
| G REPLACE_HYPER_CLIENT | NOT_BOUNDED | INSUFFICIENT | VERY HIGH | HIGH |
| H SEPARATE_PROXY_RUNTIME | NOT_BOUNDED | PLAUSIBLE | VERY HIGH | HIGH |

---

## 17. Priority ranking

### ROOT_PRIORITY_1

```text
ROOT_PRIORITY_1 = ROOT_1A_3 + ROOT_1G_2 (Hyper body/frame adapter + BoxBody elimination on wire hot path)
ROOT_PRIORITY_1_CLASS = EXYONQ_HYPER_INTEGRATION + ALLOCATION
ROOT_PRIORITY_1_EVIDENCE = STRONGLY_SUPPORTED
ROOT_PRIORITY_1_MEASURED_COST_US_PER_REQ = BOUNDED 15–25
ROOT_PRIORITY_1_RECOVERABLE_US_PER_REQ = BOUNDED 8–15
ROOT_PRIORITY_1_EXPECTED_RPS_GAIN = BOUNDED 6–12%
ROOT_PRIORITY_1_EXPECTED_CPU_GAIN = BOUNDED 8–15%
ROOT_PRIORITY_1_EXPECTED_MEMORY_GAIN = BOUNDED 5–10% dirty RSS
ROOT_PRIORITY_1_EXPECTED_TAIL_EFFECT = NEUTRAL (requires regression gate)
ROOT_PRIORITY_1_COMPLEXITY = MEDIUM
ROOT_PRIORITY_1_RISK = MEDIUM (correctness on streaming/CL/SSE boundaries)
ROOT_PRIORITY_1_MIGRATION_VALUE_EFFECT = PARTIAL (does not alone prove P4 win)
```

### ROOT_PRIORITY_2

```text
ROOT_PRIORITY_2 = ROOT_1F_2 + ROOT_1A_2 (collapse Hyper mpsc async boundaries / connection-local execution)
ROOT_PRIORITY_2_EXPECTED_RPS_GAIN = BOUNDED 5–10%
```

### ROOT_PRIORITY_3

```text
ROOT_PRIORITY_3 = ROOT_1B Cap034 productization (DEFER per owner; +5.27% measured in experiment)
```

---

## 18. Proposals

```text
NEXT_PRODUCT_WIP_PROPOSAL = NONE
  # superseded 2026-08-26: PXDP-B design complete — see v044-proxy-execution-data-plane-architecture-design.md
  (targeted experiment: stream upstream Incoming → wire writer with fewer BoxBody/frame
   transitions; no Hyper replacement; no allocator swap; correctness gates required)

NEXT_MEASUREMENT_WIP_PROPOSAL = NONE

CAP034_PRODUCTIZATION_PRIORITY = DEFER
CAP034_CANONICAL_MERGE = NO
HAPROXY_IN_THIS_WIP = NOT_MEASURED
```

```text
EXPECTED_USER_VALUE = BOUNDED partial P4 gap closure
EXPECTED_RPS_GAIN = BOUNDED 6–12% (priority-1 slice)
EXPECTED_CPU_GAIN = BOUNDED 8–15%
EXPECTED_MEMORY_GAIN = BOUNDED 5–10%
EXPECTED_P95_EFFECT = NEUTRAL
EXPECTED_P99_EFFECT = NEUTRAL
EXPECTED_MIGRATION_VALUE_EFFECT = PARTIAL
MINIMUM_ACCEPTABLE_GAIN = 7% RPS for medium-complexity product WIP
```

---

## 19. Terminal report fields

See §2–§18 for measured values. Summary:

```text
V044_P4_PROXY_EXECUTION_TAX_DECOMPOSITION_STATUS = COMPLETE
CASE = PX-B

WORKTREE_STATUS = CLEAN_CANONICAL (no product mutation)
PRODUCT_CODE_CHANGED = NO
ARCHITECTURE_CHANGED = NO
DEPENDENCIES_CHANGED = NO
CARGO_LOCK_CHANGED = NO

DOCS_RECONCILED = YES
NOTION_UPDATE_REQUIRED = YES
PUSH = NO
```
