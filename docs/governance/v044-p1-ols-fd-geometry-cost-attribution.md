# V044 P1 — OLS FD geometry CPU cost attribution

```text
STATUS = COMPLETE
CASE = A
CURRENT_WIP = V044_P1_OLS_FD_GEOMETRY_COST_ATTRIBUTION  # sealed; do not reopen
MODE = READ_ONLY_CAUSAL_MEASUREMENT
PRODUCT_MUTATION = NO
FD_REUSE_PRODUCT_REPAIR = AUTHORIZED_AND_COMPLETED
  # successor: docs/governance/v044-p1-fd-reuse-repair.md (CASE R1)
DESIGN_IMPLEMENTATION = FORBIDDEN_IN_THIS_WIP
P2_STARTED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

ROOT3_PRODUCT_COMMIT = 2462410ec93fc54d50e6ef1f977d71e17f370450
EXYONQ_BINARY_SHA256 = f243ef2c07a41c88c14a86c1241598051a24d0eeac891f96c319b9ab5ce49f1d
EVIDENCE_HOST = Netcup amd64 (netcup-bench)
EVIDENCE = .exyonq-local-evidence/v044-p1-ols-fd-attr-v3-20260821T114904Z/
HARNESS = scripts/remote/v044-p1-ols-fd-geometry-cost-attribution.sh
```

Successor (do not rewrite this CASE A meter): FD-reuse repair R1 is in
[v044-p1-fd-reuse-repair.md](v044-p1-fd-reuse-repair.md). The **4.35 µs/req**
userspace residual below is the **pre-repair** cgroup attribution. It is **not**
the current post-repair residual and must not be reused as a new optimization
target. This document does **not** claim Oracle ARM64 REAL_PRODUCTION; ARM64
library tests ≠ real Cap067 E2E (see the repair closeout section).

## Phase 1 — Live comparability

| Check | Result |
|--|--|
| ExyonQ binary SHA-256 | `f243ef2c…5ce49f1d` MATCH sealed Root1+Root3 |
| OLS container | RUNNING_HEALTHY · `:8088/site/1k.bin` 200/1024 |
| ExyonQ / OLS cpuset | `0-7` / `0-7` |
| ExyonQ / OLS CpuQuota | `0` / `0` |
| P1 contract | STATIC_1K_HTTP11_KEEPALIVE_100C_2THREAD_20S_WARMUP_30S_MEASURE |
| CPU accounting | cgroup v2 `cpu.stat` exposes `user_usec` + `system_usec` for both |

```text
CPU_ACCOUNTING_COMPARABILITY = PASS
```

Live three-way authority (docker-stats core-ms model) remains:

```text
EXYONQ_CORE_MS_PER_REQ = 0.013296
OLS_CORE_MS_PER_REQ    = 0.010342
EXYONQ_CPU_COST_VS_OLS_PERCENT = +28.56%
```

This WIP attributes **µs/request** with cgroup + bpftrace + perf. Absolute µs from cgroup and core-ms from docker-stats are **different meters** — do not force them to identical µs.

## Phase 2 — User / system CPU µs per request

**Method (authoritative for this WIP):** Phase A — warm 20s, measure 30s, **no** perf/bpf observers.

```text
source = cgroup v2 cpu.stat {user,user,system}_usec delta / rewrk requests_total
```

| | ExyonQ | OLS |
|--|--|--|
| RPS (this window) | 173710 | 128805 |
| USER µs/req | **8.600** | **4.250** |
| SYSTEM µs/req | **17.008** | **10.956** |
| TOTAL µs/req | **25.608** | **15.206** |

```text
TOTAL_CPU_GAP_US_PER_REQ   = 10.402
SYSTEM_CPU_GAP_US_PER_REQ  =  6.051
USER_CPU_GAP_US_PER_REQ    =  4.350
SYSTEM_GAP_PERCENT_OF_TOTAL = 58.2%
USER_GAP_PERCENT_OF_TOTAL   = 41.8%
```

Interpretation: the ExyonQ–OLS gap is **majority system**, with a still-material **userspace** share.

## Phase 3 — Syscall cost attribution

**Method:** Phase B — bpftrace enter/exit `sum(nsecs)` for container TGIDs during a separate 30s load (BOUNDED wall-time-in-syscall; may include brief blocking). Counts / req confirm geometry.

| Syscall | ExyonQ calls/req | ExyonQ µs/req (bpf wall) | OLS calls/req | OLS µs/req (bpf wall) |
|--|--|--|--|--|
| openat2 / openat | **1.000** | **3.760** | 0.000067 | 0.005 |
| statx | **1.000** | **1.687** | ~0 | ~0 |
| close | **1.388** | **1.222** | 0.000175 | 0.001 |
| sendfile64 | **1.000** | **5.993** | 0 | 0 |
| writev | 0 | 0 | **1.000** | **6.899** |

```text
OPENAT2_CPU_US_PER_REQ = 3.760   # BOUNDED (bpf wall)
STATX_CPU_US_PER_REQ   = 1.687   # BOUNDED
CLOSE_CPU_US_PER_REQ   = 1.222   # BOUNDED
OLS_OPEN_CPU_US_PER_REQ  = 0.005
OLS_CLOSE_CPU_US_PER_REQ = 0.001
OLS_WRITEV_CPU_US_PER_REQ = 6.899
```

**Cross-check (perf cycles, children % of all samples × total cgroup µs/req):**

| Kernel entry | Children % | Est. µs/req (on-CPU) |
|--|--|--|
| `__x64_sys_openat2` → `do_sys_openat2` → `path_openat` | 13.36% | **3.42** |
| `__x64_sys_statx` → `vfs_statx` | 5.61% | **1.44** |
| `__fput` / close path (approx bundle) | ~3% | **~0.77** |
| `__x64_sys_sendfile64` (includes TCP TX under splice) | 17.16% | 4.40 |
| OLS `__x64_sys_writev` (includes TCP TX) | 47.96% | (OLS-scale) |

openat2/statx bpf wall ≈ perf on-CPU (good agreement). close bpf slightly higher than perf self (BOUNDED).

## Phase 4 — VFS / path-resolution bounds

Cap004-safe `openat2(RESOLVE_BENEATH)` work lands under `do_sys_openat2` / `path_openat` (perf children 11.08% under openat2). Observed supporting symbols: `override_creds`, `apparmor_file_open`, `prepend_path`, `lockref_*`.

Avoid double-count with nested children:

```text
PATH_RESOLUTION_US_PER_REQ     ≈ 3.42–3.76   # openat2 path (BOUNDED)
METADATA_US_PER_REQ            ≈ 1.44–1.69   # statx (BOUNDED)
FD_OPEN_LIFECYCLE_US_PER_REQ   ⊆ PATH_RESOLUTION (fd_install nested)
FD_CLOSE_LIFECYCLE_US_PER_REQ  ≈ 0.77–1.22   # fput/close (BOUNDED)
OTHER_VFS_US_PER_REQ           ≈ 0           # no separate residual attributed beyond above
```

## Phase 5 — Kernel function differential

### ExyonQ (self %, cycles)

| Symbol | self% | est. µs/req | Category |
|--|--|--|--|
| `__raw_callee_save___pv_queued_spin_unlock` | 4.96 | 1.27 | SCHEDULER |
| `_raw_spin_lock_bh` | 2.82 | 0.72 | SCHEDULER |
| `__fdget` | 2.12 | 0.54 | FD_LIFECYCLE |
| `epoll_sendfile_eligible` (userspace) | 1.54 | 0.39 | SENDFILE |
| `fput` / `__fput` | 1.43 / 1.29 | 0.37 / 0.33 | FD_LIFECYCLE |
| `override_creds` | 1.15 | 0.29 | PATH_LOOKUP |
| `apparmor_file_open` | 0.93 | 0.24 | PATH_LOOKUP |
| `prepend_path` | 0.92 | 0.24 | PATH_LOOKUP |

Children tree confirms **openat2 + statx** as major syscall roots; sendfile children are mostly **TCP transmit**, not VFS open.

### OLS (self % / children)

| Symbol | Note | Category |
|--|--|--|
| `__x64_sys_writev` | **47.96%** children — body path | WRITEV / TCP |
| openat / statx | ~0 in bpf + absent as major children | PATH_LOOKUP ≈ 0 |
| spin unlock / nf_conntrack / skb | top self | TCP / SCHEDULER |

Extra ExyonQ system CPU **does** land in the expected VFS/open/stat/close family (`do_sys_openat2`, `path_openat`, `vfs_statx`, `__fdget`, `fput`).

## Phase 6 — Bounded cost model

```text
TOTAL_EXYONQ_VS_OLS_GAP_US_PER_REQ = 10.402   # PROVEN (cgroup Phase A)

OPENAT2_ATTRIBUTED_US_PER_REQ = 3.42–3.76     # BOUNDED (perf–bpf)
STATX_ATTRIBUTED_US_PER_REQ   = 1.44–1.69     # BOUNDED
CLOSE_ATTRIBUTED_US_PER_REQ   = 0.77–1.22     # BOUNDED
OTHER_VFS_ATTRIBUTED_US_PER_REQ = 0           # UNKNOWN / not separated

FD_GEOMETRY_TOTAL_US_PER_REQ =
  LOWER = 5.63   # perf children openat2+statx+fput bundle
  UPPER = 6.05   # min(bpf_wall_fd_diff 6.66, SYSTEM_CPU_GAP 6.05)
  # Confidence = BOUNDED

SENDFILE_VS_WRITEV_DIFFERENTIAL_US_PER_REQ = -0.91
  # bpf wall: sendfile64 5.99 − writev 6.90
  # ExyonQ body path is NOT more expensive than OLS writev on this meter
  # Confidence = PARTIAL (both dominated by TCP under the syscall)

USERSPACE_DIFFERENTIAL_US_PER_REQ = 4.350     # PROVEN (cgroup user gap)

UNEXPLAINED_US_PER_REQ ≈ 4.35                 # after assigning FD upper to full system gap
  # = residual user gap (+ any system not capped into FD)
  # Confidence = PARTIAL

FD_GEOMETRY_EXPLAINS_PERCENT =
  54–58% of CLEAN_CGROUP TOTAL_CPU_GAP_US_PER_REQ
  # LOWER 5.63/10.40 = 54.1%; UPPER 6.05/10.40 = 58.2%
  # Do NOT claim 100% of +28.56 docker-stats core-ms gap
  # Absolute µs scales differ between meters; status +28.56% remains live authority
```

## Decision gate — CASE A

```text
OLS_FD_REUSE_DIFFERENTIAL = PROVEN_MATERIAL
FD_GEOMETRY_EXPLAINS_PERCENT = 54–58   # of cgroup total µs/req gap; BOUNDED
PRIMARY_REMAINING_ROOT = USERSPACE_CPU_GAP (~4.35 µs/req; ~42% of cgroup gap)
  # plus any non-FD kernel (locks/TCP) not separated here
REAL_PRODUCT_EFFICIENCY_DEFECT = YES
OVERALL_CPU_DEFECT_STATUS = OPEN_PROVEN_OLS_DIFFERENTIAL__FD_GEOMETRY_MATERIAL
FD_REUSE_PRODUCT_REPAIR = NOT_AUTHORIZED
PRODUCT_MUTATION = NO
```

FD geometry explains a **material** share of the measured ExyonQ–OLS CPU gap (majority of the **system** half). Body sendfile-vs-writev is **not** the ExyonQ disadvantage. Userspace remains a large residual.

## Design review only (NOT implemented)

```text
DESIGN_PROBLEM =
  Cap067 P1 pays openat2(RESOLVE_BENEATH)+statx+close per successful static request
  (~5.6–6.1 µs/req BOUNDED), while OLS keeps a hot FD and only writev (~0 open/stat).
  That differential is PROVEN_MATERIAL for ~54–58% of the clean cgroup CPU gap.

EXISTING_PATTERNS_IN_SCOPE =
  Cap004 openat2(RESOLVE_BENEATH);
  Cap067 sendfile handle registry (Hyper path — not hot Cap067 epoll P1);
  generation-scoped StaticRuntime / reload publish;
  Root1 worker LocalRuntimePin;
  ADR-021 pattern discipline.

INVARIANTS = [
  Cap004 containment,
  openat2 RESOLVE_BENEATH,
  generation correctness,
  reload correctness,
  root rebind,
  no stale FD,
  no wrong-inode serving,
  no symlink escape,
  no FD leak,
  no use-after-close,
  correct replacement/deletion semantics
]

CHARACTERIZATION_GAPS = [
  Exact Cap067 epoll open/stat/close call sites vs Hyper registry path,
  close 1.39/req (extra closes beyond 1.0 open) — need site attribution,
  Userspace 4.35 µs/req residual not yet decomposed,
  Docker-stats core-ms vs cgroup µs scale reconciliation optional follow-up
]

PATTERN_SELECTED = NONE
```

### ALTERNATIVE_A — generation-scoped FD cache

| Aspect | Evaluation |
|--|--|
| Ownership | Shared cache keyed by (generation, root_id, inode/path identity) |
| Lifetime | Bound to StaticRuntime generation; drop on reload/root rebind |
| Generation invalidation | Mandatory on publish; no cross-generation FD |
| Reload | Must not serve pre-reload descriptors |
| FD pressure | Global FD count growth; needs hard caps + eviction |
| Memory | Per-entry inode/path metadata |
| Thread locality | Contention on shared map |
| Security | Must retain Cap004 open-at-cache-miss; never cache raw path without inode/gen |
| Failure modes | Stale FD, wrong inode, symlink escape if validation weak |
| Expected CPU benefit | Up to ~FD_GEOMETRY upper (~6 µs/req) if hit rate ≈1 |
| Complexity | High |

### ALTERNATIVE_B — worker-local generation-scoped FD cache

| Aspect | Evaluation |
|--|--|
| Ownership | Per-worker LocalRuntimePin-adjacent map |
| Lifetime | Worker + generation |
| Generation invalidation | Local drop on generation observe |
| Reload | Same as A, local |
| FD pressure | Multiplied by worker count |
| Memory | Higher than A for same keys |
| Thread locality | Better (matches Root1 pin) |
| Security | Same Cap004/inode/gen rules |
| Failure modes | Same class; harder to audit N caches |
| Expected CPU benefit | Similar hit-rate limited |
| Complexity | High (N caches + invalidation) |

### NO_NEW_PATTERN_ALTERNATIVE — keep openat2/statx/close per request

| Aspect | Evaluation |
|--|--|
| Ownership | None |
| Security | Strongest (always RESOLVE_BENEATH) |
| Expected CPU benefit | 0 vs today |
| Complexity | 0 |
| Fit | Accept OLS gap unless owner authorizes cache work |

### Forbidden (any future work)

- Disabling Cap004 / removing `RESOLVE_BENEATH`
- Caching raw paths without generation + inode validation
- Serving stale FDs across root/generation transitions
- Benchmark-only or P1-only hidden FD cache
- Fake/synthetic cache tests as product proof

## Terminal (this WIP)

```text
P1_OLS_FD_GEOMETRY_COST_ATTRIBUTION_STATUS = COMPLETE
CPU_ACCOUNTING_COMPARABILITY = PASS
OLS_FD_REUSE_DIFFERENTIAL = PROVEN_MATERIAL
FD_GEOMETRY_EXPLAINS_PERCENT = 54-58
FD_REUSE_PRODUCT_REPAIR = NOT_AUTHORIZED
PRODUCT_MUTATION = NO
CARGO_LOCK_CHANGED = NO
P1_THROUGHPUT_STATUS = CLOSED_CURRENT_COMPETITIVE_BASELINE
P1_CPU_EFFICIENCY_VS_NGINX = CLOSED_COMPETITIVE
P1_CPU_EFFICIENCY_VS_OLS = MATERIAL_GAP
P2_STARTED = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
```

STOP for owner authorization before any FD-reuse product work.
