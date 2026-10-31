# V044 P4 — Proxy path causal profiling

```text
STATUS = COMPLETE
CASE = P4P-B
WIP = V044_P4_PROXY_PATH_CAUSAL_PROFILING
MODE = READ_ONLY_CAUSAL_PROFILING
PRODUCT_MUTATION = NO
ARCHITECTURE_MUTATION = NO
P5_STARTED = NO
P5_TO_P14_FROZEN = YES
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

HAPROXY_GOVERNANCE_RECONCILIATION = COMPLETE

SOURCE_HEAD = ade997a362e60d1dc6eafc984da454930600845a
SOURCE_TREE = 4ae6ffc7528d0881be23fcac246c345dcb9a1606
PRODUCT_BINARY_SHA256 = e28c0abe18587f2d911d492be64cbec42372383e011417a6c578be52da5ba459

RUN_ID = v044-p4-proxy-path-causal-profiling-20260825T203611Z
EVIDENCE_NETCUP =
  /root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p4-proxy-path-causal-profiling-20260825T203611Z/
HARNESS = scripts/remote/v044-p4-proxy-path-causal-profiling.sh
BASELINE_RUN = v044-p4-proxy-1k-3way-20260822T115113Z
HOST = Netcup amd64 (netcup-bench)
```

## Part A — HAProxy governance (prerequisite)

```text
DOCS_COMPARATOR_SET = NGINX / OPENLITESPEED / HAPROXY
DASHBOARD_SUPPORTS_HAPROXY = YES
P1_HAPROXY = NOT_MEASURED
P2_HAPROXY = NOT_MEASURED
P3_HAPROXY = NOT_MEASURED
P4_HAPROXY = NOT_MEASURED
HISTORICAL_BENCHMARKS_ARE_THREE_WAY = YES
```

Scoreboard: `docs/benchmarks/v044-p1-p14-competitive-dashboard.json` — HAProxy columns
render `NOT_MEASURED` for P1–P4. No retroactive HAProxy numbers fabricated.

---

## Part B — Profiling contract

```text
P4_PROFILE_CONTRACT_PRESERVED = YES
P4_VALIDITY = PASS
P4_COMPARABILITY = PASS (with disclosed wire asymmetry below)

BENCH_CONTRACT = P4_PROXY_1K /api/ 1024B upstream keepalive / RAW WAF off
CPUSET = 0-7
CLIENT_CONCURRENCY = 100
LOAD_THREADS = 2
WARMUP = 20s
MEASURE = 30s
WORKER_GEOMETRY = 8/8/8 (product defaults)
PRODUCT_MUTATION = NO
```

Invalid run `112732Z` excluded from all causal comparison.

---

## 1. perf stat (steady-state load)

Profiling-session RPS (same contract, not rebaseline): ExyonQ **76215** · NGINX **91892** · OLS **93966**.

| Metric | ExyonQ | NGINX | OLS |
| --- | ---: | ---: | ---: |
| cycles/req | **34,617,095** | 2,037,055 | 1,676,263 |
| instructions/req | **25,460,815** | 2,109,979 | 1,979,885 |
| IPC | **0.735** | 1.036 | 1.181 |
| ctx switches/req | **115.78** | 8.72 | 8.75 |
| cpu migrations/req | 0.552 | 0.009 | 0.010 |
| page faults/req | 8.89 | 43.38 | 0.0015 |

```text
EXYONQ_EXCESS_CYCLES_VS_NGINX = +1599%
EXYONQ_EXCESS_INSTRUCTIONS_VS_NGINX = +1107%
CTX_SWITCH_RATIO_EXYONQ_VS_NGINX = 13.3×
```

ExyonQ executes **~12× more instructions** and **~17× more cycles** per request than NGINX
on the same P4 contract, with **~13× more context switches per request**.

---

## 2. perf record — top ExyonQ symbols (exclusive, tokio-rt-worker sample)

| % self | Symbol / class |
| ---: | --- |
| 4.89% | kernel IPI / reschedule |
| 3.75% | **malloc** |
| 1.53% | **bytes::shared_drop** |
| 1.24% | **bytes::shared_v_drop** |
| 1.06% | **bytes::shared_clone** |
| 0.91% | **tokio::io::Registration::poll_ready** |
| 0.83% | **hyper_util::server::conn::auto::poll** |
| 0.75% | **hyper client pool checkout** |
| 0.64% | **hyper mpsc unbounded send (dispatch)** |
| 0.58% | **hyper client send_request** |
| 0.53% | exyonq_core::server::accept_loop |
| 0.53% | exyonq_core::server::serve_connection |

Inclusive stacks additionally show heavy **kernel sendto/tcp_write_xmit** (~43% children)
on the sampled worker — consistent with userspace-driven socket churn.

---

## 3. Architectural cost-class aggregation (estimated)

| Class | Evidence | Est. exclusive % | Notes |
| --- | --- | ---: | --- |
| TOKIO_SCHEDULER / IO wake | perf + ctx switches | **STRONGLY_SUPPORTED** | poll_ready, 13× ctx switches/req |
| HYPER_CLIENT | perf symbols | **STRONGLY_SUPPORTED** | pool checkout, send_request, mpsc |
| HYPER_HTTP_PARSE / server conn | perf symbols | **BOUNDED** | hyper_util server auto poll |
| ALLOCATOR | malloc 3.75% + cfree | **STRONGLY_SUPPORTED** | |
| MEMCPY / bytes churn | shared_clone/drop ~3.8% | **STRONGLY_SUPPORTED** | |
| CHUNKED_ENCODING | wire + writev path | **STRONGLY_SUPPORTED** | see §7 |
| CLIENT_WRITE | kernel sendto inclusive | **STRONGLY_SUPPORTED** | |
| UPSTREAM_READ | recvfrom strace | **OBSERVED** | ~0.71 recvfrom/req (worker sample) |
| CLUSTER_RWLOCK | not in top symbols | **REJECTED_NOT_MATERIAL** | this sample |
| LOCKING (futex) | strace | **BOUNDED** | ~0.11 futex/req on worker |

```text
HYPER_CLIENT_COST = STRONGLY_SUPPORTED
CHUNKED_ENCODING_COST = STRONGLY_SUPPORTED
UPSTREAM_POOL_COST = INSUFFICIENT_EVIDENCE (partial external obs only)
CLUSTER_LOOKUP_COST = REJECTED_NOT_MATERIAL (this sample)
SCHEDULER_HANDOFF_COST = STRONGLY_SUPPORTED
BUFFER_COPY_COST = STRONGLY_SUPPORTED (bytes refcount path)
ALLOCATOR_COST = STRONGLY_SUPPORTED
```

---

## 4. Upstream pool reuse

```text
UPSTREAM_CONNECTION_REUSE_METRICS = PARTIAL
PRODUCT_COUNTER_INSTRUMENTATION = FORBIDDEN (not used)
```

External observation under load:

- **NGINX → upstream:** ~100 `TIME_WAIT` sockets + 1 `ESTABLISHED` during sample window
  → keepalive pool active but material close/reopen churn visible under `rewrk -c 100`.
- **ExyonQ → upstream:** no established sockets captured in quick `ss` snapshot (timing);
  upstream strace on one worker: **115 close** in 10s, **0 connect** (connects on other threads).

```text
IS_UPSTREAM_POOL_CHURN_PRESENT = YES (NGINX OBSERVED; ExyonQ PARTIAL)
IS_POOL_REUSE_COMPARABLE_TO_NGINX = INSUFFICIENT_EVIDENCE
POOL_HIT_RATE = NOT_MEASURED
POOL_MISS_RATE = NOT_MEASURED
UPSTREAM_REUSE_RATIO = NOT_MEASURED
```

Cannot close `IS_UPSTREAM_POOLING_OR_PROXY_STATE_MACHINE_INEFFICIENT` — remains
**INSUFFICIENT_EVIDENCE** for ExyonQ-specific pool economics.

---

## 5. Syscall geometry (12s strace, single ExyonQ worker)

Estimated ~914k requests in window at profiling RPS.

| Syscall | calls/req (worker) |
| --- | ---: |
| **total** | ~1.55 |
| recvfrom | 0.71 |
| sendto | 0.36 |
| writev | 0.36 |
| futex | 0.11 |
| epoll_wait | 0.015 |

NGINX worker (10s): ~1 writev/req + ~1 recvfrom/req — simpler 1:1 geometry.

```text
COMPARATOR_ASYMMETRY = ExyonQ uses sendto+writev split; rivals primarily writev+recvfrom
FUTEX_PER_REQUEST = ~0.11 (ExyonQ worker sample)
```

---

## 6. Scheduler / handoff

```text
P4_EXYONQ_CONTEXT_SWITCHES_PER_REQ = 115.78
P4_NGINX_CONTEXT_SWITCHES_PER_REQ = 8.72
P4_OLS_CONTEXT_SWITCHES_PER_REQ = 8.75

P4_EXYONQ_CPU_MIGRATIONS_PER_REQ = 0.552

WORKER_IMBALANCE = BOUNDED (18 tokio threads; sample on one worker)
TASK_MIGRATION_COST = STRONGLY_SUPPORTED (ctx switch ratio 13×)
```

Does **not** reopen G1 worker-count root — issue is per-request scheduler wake cost on
Tokio path, not Cap067 geometry.

---

## 7. Chunked transfer encoding (wire verified)

| Server | Transfer-Encoding | Connection |
| --- | --- | --- |
| **ExyonQ** | **chunked** | **close** |
| NGINX | (none — Content-Length: 1024) | keep-alive |
| OLS | (none — content-length: 1024) | Keep-Alive |

```text
CLIENT_RESPONSE_TRANSFER_ENCODING = CHUNKED
CHUNKS_PER_RESPONSE = NOT_MEASURED (fixed 1024 B body → small chunk count)
WRITES_PER_RESPONSE = BOUNDED ≥2 (chunk framing + body path; writev ~0.36/req worker sample)

CHUNK_ENCODING_CPU_COST = STRONGLY_SUPPORTED
COMPARATOR_ASYMMETRY = YES — disclosed; rivals use Content-Length + client keepalive
CONTRACT_SOURCE = Cap034 one-shot wire + wire_conn.rs forces Connection: close + TE chunked
```

---

## 8. Dual HTTP stack

```text
CLIENT_HTTP_PARSE = BOUNDED (wire path avoids full Hyper server on hot path; some hyper_util server poll visible)
UPSTREAM_HTTP_PARSE = STRONGLY_SUPPORTED (Hyper client framing)
UPSTREAM_REQUEST_ENCODE = STRONGLY_SUPPORTED (Hyper dispatch mpsc)
CLIENT_RESPONSE_ENCODE = STRONGLY_SUPPORTED (chunked TE progressive write)

HYPER_COST = STRONGLY_SUPPORTED
EXYONQ_GLUE_COST = STRONGLY_SUPPORTED (bytes, tokio poll, accept/serve_connection)
PROTOCOL_NECESSARY_COST = BOUNDED (proxy requires upstream+client HTTP)
AVOIDABLE_DUPLICATE_COST = BOUNDED (chunked+close vs CL+keepalive asymmetry)
```

---

## 9. Memory / PSS (profiling load snapshot)

| | ExyonQ | NGINX | OLS |
| --- | ---: | ---: | ---: |
| PSS MiB | **46.95** | 3.55 | 1.84 |
| Private_Clean MiB | 11.26 | 3.10 | 1.20 |
| Private_Dirty MiB | **35.13** | 0.15 | 0.26 |

```text
P4_BASE_IMAGE_MIB ≈ 11.3  (Private_Clean)
P4_PROXY_RUNTIME_STATE_MIB ≈ 24–35  (Private_Dirty under load; Hyper/Tokio/upstream)
P4_BUFFER_POOL_MIB = NOT_MEASURED
P4_UNEXPLAINED_MEMORY_MIB ≈ 0–5 (PSS meter vs docker-stats baseline drift)
P4_RECOVERABLE_MEMORY_MIB = NOT_BOUNDED
```

Note: profiling snapshot PSS **46.95 MiB** > baseline docker RSS **33.18 MiB** — load-state
delta; ranking vs NGINX preserved.

---

## 10. ROOT_1 decomposition

### ROOT_1A — Hyper + Tokio proxy runtime stack

```text
EVIDENCE_STRENGTH = STRONGLY_SUPPORTED
MEASURED_COST = +1107% instructions/req vs NGINX; hyper pool/send/mpsc in top symbols
RECOVERABLE_COST = NOT_BOUNDED
EXPECTED_RPS_GAIN = NOT_BOUNDED
RECOMMENDATION = TARGETED_PROXY_PATH_EVOLUTION (future; not authorized here)
```

### ROOT_1B — Chunked TE + Connection: close (Cap034 contract)

```text
EVIDENCE_STRENGTH = STRONGLY_SUPPORTED (wire PROVEN)
MEASURED_COST = STRONGLY_SUPPORTED class; µs/request NOT_ISOLATED
RECOVERABLE_COST = NOT_BOUNDED (requires contract/product change to CL+keepalive)
COMPARATOR_ASYMMETRY = YES
RECOMMENDATION = OWNER_REVIEW contract vs competitive parity
```

### ROOT_1F — Scheduler / context-switch excess

```text
EVIDENCE_STRENGTH = STRONGLY_SUPPORTED
MEASURED_COST = 115.8 vs 8.7 ctx switches/req (13.3×)
RECOVERABLE_COST = NOT_BOUNDED
RECOMMENDATION = MEASURE_MORE only if pursuing Tokio wake reduction
```

### ROOT_1G — Allocator + bytes refcount churn

```text
EVIDENCE_STRENGTH = STRONGLY_SUPPORTED
MEASURED_COST = malloc 3.75% + bytes clone/drop ~3.8% exclusive
RECOVERABLE_COST = NOT_BOUNDED
RECOMMENDATION = subset of ROOT_1A evolution
```

### ROOT_1C — Upstream pool

```text
EVIDENCE_STRENGTH = INSUFFICIENT_EVIDENCE
```

### ROOT_1E — Cluster RwLock

```text
EVIDENCE_STRENGTH = REJECTED_NOT_MATERIAL (this profiling sample)
```

---

## 11. Rank by recoverable product value

```text
ROOT_1_SUBROOT_PRIORITY_1 = ROOT_1B_CHUNKED_TE_CONNECTION_CLOSE
  # PROVEN wire + contract; largest comparator asymmetry; recoverable only via product contract change

ROOT_1_SUBROOT_PRIORITY_2 = ROOT_1A_HYPER_TOKIO_RUNTIME_STACK
  # STRONGLY_SUPPORTED 12× instruction gap; recoverable fraction NOT_BOUNDED

ROOT_1_SUBROOT_PRIORITY_3 = ROOT_1F_SCHEDULER_CONTEXT_SWITCH_EXCESS
  # STRONGLY_SUPPORTED 13× ctx switches/req; recoverable NOT_BOUNDED
```

```text
RECOVERABLE_PROXY_CPU_PERCENT = NOT_BOUNDED
RECOVERABLE_PROXY_CPU_US_PER_REQ = NOT_BOUNDED
```

Does **not** meet future product WIP bar (`RECOVERABLE_COST = BOUNDED` required).

---

## 12. Architecture classification

```text
PROXY_ARCHITECTURE_FITNESS = TARGETED_PROXY_PATH_EVOLUTION

IS_HYPER_INTRINSICALLY_THE_PRIMARY_ROOT = BOUNDED
IS_EXYONQ_HYPER_INTEGRATION_THE_PRIMARY_ROOT = STRONGLY_SUPPORTED
IS_CHUNKED_TE_A_MATERIAL_ROOT = STRONGLY_SUPPORTED
IS_UPSTREAM_POOLING_A_MATERIAL_ROOT = INSUFFICIENT_EVIDENCE
IS_PROXY_SHARED_STATE_A_MATERIAL_ROOT = REJECTED_NOT_MATERIAL
IS_SCHEDULER_HANDOFF_A_MATERIAL_ROOT = STRONGLY_SUPPORTED
IS_BUFFER_COPY_GEOMETRY_A_MATERIAL_ROOT = STRONGLY_SUPPORTED
```

Primary answer to “where does extra user CPU go?”:

```text
ANSWER = Tokio wake/scheduling (13× ctx switches) + Hyper client/upstream framing
         + bytes allocator/refcount churn + contract-mandated chunked/close client framing
         vs rivals' Content-Length/keep-alive path — NOT a single symbol
```

---

## 13. Terminal

```text
V044_P4_PROXY_PATH_CAUSAL_PROFILING_STATUS = COMPLETE
CASE = P4P-B

NEXT_PRODUCT_WIP_PROPOSAL = NONE
  # recoverable CPU not bounded; Cap034 TE/close change is product semantics — owner only
  # see v044-cap034-proxy-framing-keepalive-contract-review.md (CASE C34-C)

NEXT_MEASUREMENT_WIP_PROPOSAL = NONE
  # superseded by V044_P4_PROXY_EXECUTION_TAX_DECOMPOSITION — CASE PX-B (20260825T230314Z)

SUCCESSOR_DECOMPOSITION = docs/governance/v044-p4-proxy-execution-tax-decomposition.md
SUCCESSOR_CASE = PX-B
SUCCESSOR_EVIDENCE = v044-p4-proxy-execution-tax-decomposition-20260825T230314Z

ROOT_1B_CAP034_SLICE_BOUNDED = YES
ROOT_1B_CAP034_RPS_RECOVERABLE_PERCENT ≈ 5.3
ROOT_1B_CAP034_DOC = docs/governance/v044-cap034-framing-keepalive-bounded-value-ab.md

GLOBAL_MIGRATION_VALUE = NOT_PROVEN
MARKET_READINESS = NOT_READY
DOCS_RECONCILED = YES
NOTION_UPDATE_REQUIRED = YES
```
