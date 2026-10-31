# V044 P1 — CPU efficiency causal analysis

```text
CURRENT_WIP = V044_P1_CPU_EFFICIENCY_CAUSAL_ANALYSIS
MODE = DIAGNOSTIC_FIRST
PRODUCT_MUTATION = NO
OPTIMIZATION_AUTHORIZED = NO
P1_THROUGHPUT_STATUS = CLOSED_CURRENT_COMPETITIVE_BASELINE
P1_CPU_EFFICIENCY_STATUS = OPEN_CAUSAL_REVIEW_REQUIRED
P2_STARTED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
```

**Evidence:** `.exyonq-local/benchmarks/v044-p1-cpu-efficiency-20260820T235807Z/`  
**Harness:** `scripts/remote/v044-p1-cpu-efficiency-causal.sh`  
**Product binary SHA-256:** `5a31ccf3…0a13c40` (unchanged)  
**Host:** Netcup amd64 · compose `v044p1auth-clean` · P1 RAW WAF-off

---

## Headline (reporting capture — authoritative efficiency)

| | ExyonQ | NGINX | OLS |
| --- | --- | --- | --- |
| RPS | 163858.57 | 148585.55 | 145136.51 |
| CPU avg % | 413.08 | 280.54 | 189.66 |
| Req / CPU-core | 39667.51 | 52964.12 | 76524.58 |
| Core-ms / req | 0.02521 | 0.01888 | 0.01307 |

ExyonQ CPU cost/req vs NGINX **+33.52%** · vs OLS **+92.91%**.

---

## Phase 1 — CPU accounting comparability

```text
CPU_ACCOUNTING_COMPARABILITY = PASS
```

All three containers: `CpusetCpus=0-7`, `NanoCpus=0`, `CpuQuota=0`. Docker stats = container cgroup. OLS `sleep` helper idle. Same 30s rewrk contract, concurrency 100, dual-thread loadgen.

---

## Causal result (short)

```text
CPU_EXCESS_PRIMARY_DOMAIN = USERSPACE_PRODUCT_HOTPATH   # vs NGINX
CONFIDENCE = HIGH

PRIMARY_ROOT_CAUSE = PER_REQUEST_USERSPACE_STATIC_CONTROL_PLANE
REAL_PRODUCT_EFFICIENCY_DEFECT = YES
MUST_FIX = YES
PRODUCT_OPTIMIZATION_AUTHORIZED = NO   # stop for owner
```

### vs NGINX (the +33% story)

Cgroup deltas under load (`supplemental/cgroup_user_sys.json`):

| | user µs/req | system µs/req | total µs/req |
| --- | --- | --- | --- |
| ExyonQ | **8.997** | 17.471 | 26.468 |
| NGINX | **3.652** | 19.680 | 23.332 |
| Δ | **+5.345** | **−2.209** | +3.136 |

Pidstat under load: absolute **system % tied (~276%)**; ExyonQ userspace **~151%** vs NGINX workers **~57%**.

NGINX also does `open+fstat+close ≈ 1/req` (strace `-c`). **Open/close geometry is not the NGINX differential.**

### vs OLS (the +93% story)

OLS `openat ≈ 0` / `close ≈ 0` — keeps the static FD open. ExyonQ (and NGINX) pay open/close every request. Cgroup: system gap **+8.6 µs/req** dominates vs OLS.

---

## Syscall geometry (per successful request)

| syscall | ExyonQ | NGINX | OLS |
| --- | --- | --- | --- |
| sendfile | ~1.00 | ~1.00 | (writev body) |
| open / openat2 | openat2 ~1.00 | open ~1.00 | ~0 |
| statx / fstat | statx ~1.00 | fstat ~1.00 | ~0 |
| close | ~1.02 | ~1.00 | ~0 |
| TOTAL (traced) | ~6.2 | ~5.1 | ~2.1 |

`SENDFILE_RUNTIME_EXECUTED = YES` — and substantial work remains around it (open/stat/close + userspace control plane).

---

## Perf counters (corrected for warmup window bias)

IPC: ExyonQ **1.11** · NGINX **1.43** · OLS **1.35**  
Instructions/req nearly equal ExyonQ vs NGINX; ExyonQ burns more cycles (lower IPC).

Ctx-switches/migrations: ExyonQ **not** higher than rivals → scheduler topology **not** primary vs NGINX.

---

## Flame — top userspace product symbols (self %)

| Symbol | self% |
| --- | ---: |
| `epoll_sendfile_eligible` | 1.74 |
| `reload::read_state` | 1.57 |
| `epoll_session::runtime` (Mutex+Arc clone) | 1.35 |
| `SendfileHandleRegistry::issue` | 0.99 |
| `open_under_root_prepared` | 0.79 |
| `Path::strip_prefix` | 0.76 |
| `StaticRuntime::root_for_slot` | 0.73 |
| `match_sendfile_asset` | 0.63 |
| `method_and_path` | 0.62 |

Flame self sum ≈ kernel 67% / `exyonq` bin 18% / libc 5% (absolute samples; rivals also kernel-heavy on send path).

---

## Recommended repair (not authorized yet)

~~1. Pin `Arc<StaticRuntime>` per worker — remove per-request `runtime()` Mutex+clone.~~  
**DONE — Root1** (`docs/governance/v044-p1-cpu-efficiency-repair-root1.md`): TLS pin + `PIN_EPOCH`; Netcup A/B core-ms/req **−21.6%**; overall defect still OPEN.

Remaining (not authorized in Root1):

~~1. Bypass `SendfileHandleRegistry` Mutex HashMap on in-process epoll path — pass `Arc<SendfileAsset>` directly.~~  
**TRIED — Root2** (`docs/governance/v044-p1-cpu-efficiency-repair-root2.md`): TLS pending + sentinel `0` was architecturally safe (`IN_PROCESS_EPOLL_REQUIRES_GLOBAL_HANDLE_REGISTRY = NO`) but Netcup A/B vs Root1 showed core-ms/req **−0.335%** (noise) → `ROOT2_CAUSAL_EFFECT = FALSIFIED_OR_TOO_SMALL`; **product mutation REVERTED**.

Remaining (still not authorized):

~~1. Reduce `reload::read_state` frequency on the P1 sendfile wire path where generation contract allows.~~  
**DONE — Root3** (`docs/governance/v044-p1-cpu-efficiency-repair-root3.md`): AtomicU64 generation + published WAF-off flag; Netcup A/B core-ms/req **−8.51%** vs Root1; cumulative vs original **≈ −28.3%**.

Live three-way after seal (`docs/governance/v044-p1-cpu-efficiency-live-3way.md`):

- vs NGINX: **CLOSED_COMPETITIVE** (ExyonQ core-ms/req **−2.94%**)
- vs OLS: **MATERIAL_GAP** (**+28.56%**); `OLS_FD_REUSE_DIFFERENTIAL = PROVEN_MATERIAL` — FD open/stat/close explains **54–58%** of clean cgroup µs/req gap (BOUNDED); sendfile-vs-writev is **not** the ExyonQ disadvantage; residual ~userspace **4.35 µs/req** — **FD reuse repair NOT_AUTHORIZED** (see `v044-p1-ols-fd-geometry-cost-attribution.md`)

Optional later (owner review only):

1. Generation-scoped FD reuse **with** Cap004 / `openat2(RESOLVE_BENEATH)` preserved.

Do **not** disable containment. Do **not** start P2.

---

## Status flags

```text
P1_CPU_EFFICIENCY_CAUSAL_STATUS = COMPLETE
CPU_ACCOUNTING_COMPARABILITY = PASS
REAL_PRODUCT_EFFICIENCY_DEFECT = YES
MUST_FIX = YES
PRODUCT_MUTATION = NO
P2_STARTED = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
STOP
```
