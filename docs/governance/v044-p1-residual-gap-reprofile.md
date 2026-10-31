# V044 P1 residual gap reprofile (post sendfile convergence)

**WIP:** `V044_P1_RESIDUAL_GAP_REPROFILE` (**ARCHIVED** — pre-SessionTable architecture)  
**Product mutation:** NO  
**Evidence:** `.exyonq-local/benchmarks/v044-p1-residual-reprofile-20260820T020434Z/`  
**Product fix (unchanged):** `f2e90beb` — absolute-form → Cap067 sendfile  
**Doc prior:** `docs/governance/v044-p1-static-tx-convergence.md`  
**Superseded for current ranking by:** SessionTable fix `427b1739` + `docs/governance/v044-p1-post-sessiontable-residual.md`  
Do **not** treat this document’s causal ranking (global SessionTable mutex as primary) as current truth.

## Authority preserved

| Field | Value |
| --- | --- |
| PRODUCT_FIX_COMMIT | `f2e90beb28bab1f0efac7c714ad550f12d6794ef` |
| DOC_COMMIT (prior) | `6260a35c3f9a28ddea3df68b38162d33513b0817` |
| PRODUCT_TREE_AT_FIX | `a2dec477c1417b0993f37f50dbc619a522a15fb0` |
| STATIC_TX_FUNCTIONAL_ROOT | FIXED_AND_RUNTIME_PROVEN |
| STATIC_TX_AS_PRIMARY_RPS_ROOT | NOT_CONFIRMED |
| Old `STATIC_TX_GAP_SHARE` 40–55% | FALSIFIED / materially weakened by post-fix RPS |

Post-fix RPS context (prior session, not this control): ExyonQ median ≈102259 (−3.7% vs pre ≈106199).  
The functional fix stands. It must not be reverted for RPS reasons.

## This-session control (canonical P1 matrix)

Host variance is large. **This session’s control is the instrumented reference**; do not overwrite sealed competitive claims with these medians alone.

| Server | Median RPS (5×30s) |
| --- | --- |
| ExyonQ | 69585.48 |
| NGINX | 140909.00 |
| OLS | 141585.37 |

ExyonQ vs NGINX ≈ −50.6% (this session).

## Sendfile reconfirmation (fresh)

Under absolute-form rewrk:

| Metric | Value |
| --- | --- |
| SENDFILE_RUNTIME_EXECUTED | YES |
| sendfile/req | ≈1.00 |
| openat2/req | ≈1.00 |
| sendto/req | ≈1.00 |
| futex/req | ≈5.45 |

## Syscalls/request (post-fix)

| Server | syscalls/req | Dominant |
| --- | --- | --- |
| ExyonQ | **11.478** | futex≈5.43, openat2+close≈2, sendfile+sendto+read+write≈4 |
| NGINX | **4.091** | recvfrom+writev+sendfile+close≈4 |
| OLS | **2.117** | read+writev≈2 |

Old ExyonQ ≈8.19 syscalls/req (pre-sendfile path). Multiplier vs rivals remains, but the **body copy path is gone**. Remaining ExyonQ extras are lock/futex and per-request open/close.

`SYSCALL_MULTIPLIER_AS_PRIMARY_RPS_CAUSE` → **PARTIAL** (not the old open+read+copy story).

## Corrected perf-stat (main/worker PID attach)

Comparability to pre-fix instrumented numbers is **WEAK** (method + host load). Directional:

| Metric/req | ExyonQ post | NGINX | ExyonQ pre (context) |
| --- | --- | --- | --- |
| instructions | 110383 | 87334 | 46508 |
| cycles | 87261 | 58448 | 38042 |
| branches | 20241 | 15446 | 8517 |
| context-switches | **0.912** | 0.164 | 2.624 |

Context switches/req improved vs old 2.624 but remain ~5.6× NGINX.

Off-CPU (task-clock / configured workers / 32s): ExyonQ ≈0.324 (4 workers); NGINX ≈0.520 (8 workers).

## Workers / topology

- `EXYONQ_WORKER_THREADS=4`, `EXYONQ_ACCEPT_WORKERS=4`
- Observed: 8 `tokio-rt-worker` threads; **4 active (~16% CPU)**, 4 idle
- `SO_REUSEPORT` enabled; Cap067: Tokio accept → divert → epoll keepalive/sendfile
- Global `Mutex` on Cap067 `epoll_session::sessions` SessionTable

CPU diagnostic (NOT P1 authority): 1→2 strong; 2→4 modest; **4→8 flat** (~75k).  
`PARALLELISM_CEILING = PROVEN` under current 4-worker pin.

Concurrency diagnostic (NOT P1 authority): 50–800 connections do not unlock throughput; peak near 100.  
`CONNECTION_SHARDING_LIMIT = NOT_PROVEN`.

## Perf record — new dominant cost

Top ExyonQ userspace sample:

- **~6.28%** `Mutex::lock_contended` → `exyonq_mod_static::epoll_session::sessions`

Secondary: WAF `regex_automata` pool get_slow/put_value (~1.6–1.7%).

`COST_CENTER_REMOVED_BY_F2E90BEB`: absolute-form Hyper body materialization.  
`NEW_DOMINANT_COST_CENTER`: Cap067 session mutex / futex.

## Old hypothesis falsification

| Old cause | Status |
| --- | --- |
| STATIC_TX as primary RPS root | FUNCTIONAL_DEFECT_FIXED_BUT_RPS_ATTRIBUTION_FALSIFIED_OR_INCOMPLETE |
| Syscall multiplier | PARTIAL |
| Context-switch multiplier | PARTIAL |
| CPU underutilization | CONFIRMED (4-worker pin vs 8-CPU host / NGINX 8 workers) |
| Off-CPU wait | PARTIAL |

## New residual causal ranking (from current evidence)

1. **LOCK_FUTEX_CONTENTION_CAP067_SESSION_MUTEX** — HIGH — ~6.3% cycles + ~5.4 futex/req — estimated share **25–40% (hypothesis, not sealed)** — product-optimizable YES  
2. **STATIC_METADATA_PATH** (openat2+close/req) — MEDIUM — ~2 syscalls/req vs NGINX ~0 — **10–20% hypothesis** — FD reuse **NOT_PROVEN** as required first step  
3. **WORKER_TOPOLOGY_PINNED_TO_4** — MEDIUM — 4→8 CPU flat — share UNKNOWN until `EXYONQ_WORKER_THREADS=8` diagnostic — topology product change **NOT_PROVEN**  
4. **WAF_REGEX_POOL** — MEDIUM — secondary samples — small share hypothesis  

## Decision gates (no implementation)

| Gate | Verdict |
| --- | --- |
| FD_REUSE_REQUIRED | **NOT_PROVEN** |
| WORKER_TOPOLOGY_CHANGE_REQUIRED | **NOT_PROVEN** |
| Recommended next (auth required) | Cap067 SessionTable lock elimination/sharding; then optional worker=8 diagnostic; FD reuse only if open cost remains material |

## Status

```text
P1_STATUS = OPEN_RESIDUAL_GAP_ANALYSIS
P2_STARTED = NO
PRODUCT_MUTATION = NO   # at time of this residual reprofile
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
OWNER_AUTHORIZATION_REQUIRED_BEFORE_NEXT_PRODUCT_OPTIMIZATION = YES
```

**Follow-on (authorized later):** Cap067 SessionTable lock removed in `427b1739` —
see `docs/governance/v044-p1-cap067-sessiontable-lock.md`. Hypothesis CONFIRMED;
P1 remains open vs NGINX after that fix.

Network counter attribution: **INSUFFICIENT** (no invented per-container TX truth).
