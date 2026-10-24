# V044 P1 — FD reuse product repair (Cap067)

```text
STATUS = COMPLETE
CASE = R1_PROVEN_MATERIAL_SUCCESS
WIP = V044_P1_FD_REUSE_REPAIR_1
P2_STARTED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

PRODUCT_BINARY_SHA256 =
224e5882acf723661f4d67c055853c9f3d8926ab726e05fb24dacdfda9b00e1e
  # includes LA-CAP067-FD-001 miss-fill TOCTOU fix (3750355f)

PRE_REPAIR_SEALED_SHA256 =
f243ef2c07a41c88c14a86c1241598051a24d0eeac891f96c319b9ab5ce49f1d

PRODUCT_COMMITS =
  815c8e26 (FD reuse)
  3750355f (TOCTOU insert guard)

EVIDENCE_HOST = Netcup amd64
ORACLE_ARM64_LIBRARY_TESTS = PASS (sendfile_fd_cache lib tests only — not REAL_PRODUCTION)
ORACLE_ARM64_CAP067_REAL_E2E = FAIL_PRODUCT_DEFECT
  # hit-path sendfile+FD-reuse proven; miss/NeedHyper hang on missing/ineligible GET
ORACLE_ARM64_REAL_PRODUCTION = NOT_PROVEN
GLOBAL_CLIPPY_D_WARNINGS = NOT_PROVEN_DUE_TO_PREEXISTING_UNRELATED_STATE
FD_REUSE_AFFECTED_SCOPE_CLIPPY_D_WARNINGS = PASS
EVIDENCE = .exyonq-local-evidence/v044-p1-fd-reuse-repair-20260821T134331Z/
```

## Architecture binding

```text
CAN_EXISTING_SENDFILE_HANDLE_REGISTRY_BE_THE_SINGLE_SOURCE_OF_TRUTH = NO
```

`SendfileHandleRegistry` is a short-lived match→begin handoff (`issue`/`take`), not a
generation-scoped open-FD cache. Open/stat already happened before `issue`. Root2 showed
a global Mutex HashMap is hostile to P1 CPU. Registry remains the handoff seam.

```text
PATTERN_SELECTED = worker-local generation-scoped Cap004 FD cache (extend Cap067 TLS ownership)
  # ADR-021: not a new named pattern — extends LocalRuntimePin / thread-local SessionTable style
WHY_SELECTED =
  no hot-path Mutex; generation stamp; Cap004 open on miss; pathname identity revalidation
WHY_ALTERNATIVES_REJECTED =
  reuse registry as SoT — wrong lifetime/identity;
  shared Mutex cache — Root2 contention;
  no-new-pattern — leaves proven material OLS gap
```

## Design problem

Cap067 P1 paid `openat2(RESOLVE_BENEATH)+metadata+close` ≈1/request while OLS kept a hot FD.
Attribution: **54–58%** of clean cgroup gap (**5.63–6.05 µs/req** BOUNDED).

## Contracts preserved

```text
FILE_REPLACEMENT_CURRENT_CONTRACT =
  pathname-visible: each request’s Cap004 open (or identity revalidation) must observe
  atomic rename / recreate as a new (dev,ino); ETag/validators must not 304 on stale identity.

FD_CACHE_REPLACEMENT_SEMANTICS =
  hit path: fstatat(root_dirfd, relpath, AT_SYMLINK_NOFOLLOW) identity check;
  same identity → reuse Cap004 File Arc;
  same (dev,ino) meta drift → refresh Cap020 wire via fstat on cached FD;
  inode change / missing path → Cap004 reopen.

SEMANTICS_PRESERVED = YES
CAP004_CONTAINMENT_PRESERVED = YES
RESOLVE_BENEATH_PRESERVED = YES  # on every cache miss
GENERATION_CORRECTNESS = YES     # entries stamped; bind_roots N+1 unselectable
ROOT_REBIND_CORRECTNESS = YES
```

## FD pressure

```text
FD_CACHE_SCOPE = worker-local TLS HashMap (site_slot, request_path)
FD_CACHE_MAX_ENTRIES = 64 per worker
FD_CACHE_EVICTION_POLICY = drop arbitrary entry when full
FD_CACHE_DROP_POLICY = clear_local on pin_runtime; generation mismatch removes entry
FD_CACHE_GENERATION_INVALIDATION = entry.generation != StaticRuntime::generation()
FD_CACHE_WORKER_MULTIPLIER = epoll workers (bench: 4) × ≤64 = ≤256 FDs worst case
FD_CACHE_EXPECTED_MAX_FDS = low for P1 (typically 1 hot file × workers)
```

## Implementation

| File | Change |
|--|--|
| `sendfile_fd_cache.rs` | TLS cache + metrics |
| `sendfile.rs` | `identity_under_root` (`fstatat`) |
| `epoll_session.rs` | match uses try_hit / insert; clear on pin |
| `lib.rs` | module + test hooks |

## Netcup causal results (cgroup Phase A, no observers)

| | Before (attribution) | After FD reuse |
|--|--|--|
| ExyonQ total µs/req | 25.608 | **16.179** |
| ExyonQ user µs/req | 8.600 | **4.416** |
| ExyonQ system µs/req | 17.008 | **11.762** |
| OLS total µs/req | 15.206 | 16.130 |
| Gap ExyonQ−OLS µs/req | 10.402 | **0.049** |

```text
CPU_US_PER_REQ_DELTA = -9.429   # recovered vs attribution Phase A ExyonQ
FD_REUSE_PERCENT_OF_PREDICTED_CEILING ≈ 156% of 6.05 µs upper
  # exceeds FD-only ceiling: fstatat<<openat2+statx+close; ambient variance; secondary effects
```

## Syscall geometry after (bpftrace counts / req)

| Syscall | Before | After |
|--|--|--|
| openat2 | ≈1.0 | **≈0** (absent) |
| statx | ≈1.0 | **≈0** (absent) |
| newfstatat | — | **≈1.0** (pathname revalidation) |
| close | ≈1.39 | **≈0.115** |
| sendfile64 | ≈1.0 | **≈1.0** |

## Decision

```text
FD_REUSE_REPAIR_RESULT = PROVEN_MATERIAL_SUCCESS
CASE = R1

DOCKER_STATS_3WAY_CLEAN (median 5×30s, cpuset 0-7) — **historical post-FD / pre-H1**:
  EXYONQ_RPS ≈ 180108
  NGINX_RPS ≈ 157198
  OLS_RPS ≈ 131326
  EXYONQ_CPU_COST_VS_NGINX_PERCENT_AFTER = -22.32
  EXYONQ_CPU_COST_VS_OLS_PERCENT_AFTER = +4.02
  (pre-repair sealed live-3way: vs NGINX −2.94%; vs OLS +28.56%)
  CURRENT_AUTHORITY_AFTER_H1 =
    docs/governance/v044-p1-final-post-repair-baseline.md (CASE S1)
  DO_NOT_CARRY_+4.02_AS_CURRENT = YES

OVERALL_CPU_DEFECT_STATUS_AFTER =
  FD_GEOMETRY_ROOT_CLOSED
  OLS_DOCKER_STATS_GAP_NEAR_CLOSED (+4% core-ms; was +28.56%)
  USERSPACE_RESIDUAL_NOT_OPENED_THIS_WIP
PRODUCT_MUTATION = YES (this repair only)
P2_STARTED = NO
```

## Logic audit

```text
LOGIC_AUDIT_HEAD = 3750355f
LOGIC_AUDIT_STATUS = PASS_WITH_NONBLOCKING_P3_FINDINGS
OPEN_LOGIC_P0 = 0
OPEN_LOGIC_P1 = 0
LA-CAP067-FD-001 = FIXED
```

## Tests (Netcup amd64 + Oracle arm64)

Do not interchange these labels:

| Label | Meaning | Status |
|--|--|--|
| AMD64 real production (Netcup) | live P1 / Cap067 sendfile + FD reuse on amd64 | sealed R1 (this repair WIP) |
| ARM64 library tests | `sendfile_fd_cache::*` on aarch64 | PASS (4 tests) — **not** REAL_PRODUCTION |
| ARM64 Cap067 real E2E | real `exyonq` process + real FS + real sockets | hit path PASS; miss path FAIL (see closeout) |
| Scoped clippy | `cargo clippy -p exyonq-mod-static --no-deps -- -D warnings -A dead_code` | PASS (prior WIP; `-A dead_code` is not global) |
| Authoritative workspace clippy | `cargo clippy --workspace --all-targets -- -D warnings` | NOT_PROVEN (pre-existing `clippy::type_complexity` in `module-api/src/static_wire.rs`) |

- Netcup `exyonq-mod-static` lib: **85+** incl. FD reuse + TOCTOU predicate
- Netcup `s4_prepared_wire_identity` / `cap004_symlink_escape`: PASS
- Oracle arm64 `sendfile_fd_cache::*`: **4 passed** (library only)

## V044_P1_FD_REUSE_VERIFICATION_CLOSEOUT (2026-08-21)

```text
WIP = V044_P1_FD_REUSE_VERIFICATION_CLOSEOUT
PRODUCT_OPTIMIZATION = FORBIDDEN
USERSPACE_RESIDUAL_ATTRIBUTION_STARTED = NO
  # Do not carry pre-repair USERSPACE_CPU_GAP ≈ 4.35 µs/req forward.
  # That meter is the attribution WIP (sealed CASE A), not the post-repair residual.
P2_STARTED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
```

Canonical product at closeout (unchanged):

```text
CURRENT_HEAD = daa6f8b93b4f4ea5845f0875219f60e60e2a5231
CURRENT_TREE = cdf4652d95cca55505fd682fc41b1033bcf6ab4d
AMD64_PRODUCT_BINARY_SHA256 = 224e5882acf723661f4d67c055853c9f3d8926ab726e05fb24dacdfda9b00e1e
ORACLE_ARM64_BINARY_SHA256 = f328584c82897c5c005aeb17e509037ecf4e5ab42cae38da87ee8cb8865b7659
  # git-archive of CURRENT_HEAD; arch-distinct hash is expected
```

### Part A — Oracle ARM64 real Cap067 path

Host `oracle-quasar` / `vnic-quasar`, `uname -m = aarch64`, Linux 6.17 Oracle.

Isolated tree: git-archive of CURRENT_HEAD (not the dirty lab worktree). Unique `EXYONQ_CONTROL_SOCKET` (default `/tmp/exyonq.sock` is HOST_SHARED and was occupied).

`[modules.metrics]` was **not** enabled: `ServerState.modules_enabled()` forces Hyper for all prefixed TCP dispatch and would skip Cap067. Site `GET /metrics` is also not OpenMetrics after LA-CAP054-008. TARGET_PATH oracle = kernel `sendfile` via `strace` (Yama `ptrace_scope=1` blocks `strace -p`; wrapping the child works).

Hit-path (20 keepalive GET `/hot.bin` after warmup, 64 KiB real file):

```text
ARM64_OPENAT2_CALLS_PER_REQ = 0.0
ARM64_NEWFSTATAT_CALLS_PER_REQ = 1.0
ARM64_CLOSE_CALLS_PER_REQ = 0.1
ARM64_FD_CACHE_HIT_BEHAVIOR = HIT_PATH_NO_OPENAT2_PER_REQ
sendfile_total = 20  # 20 requests; aarch64 sendfile not sendfile64
ARM64_GET_REUSE = PASS
ARM64_KEEPALIVE = PASS
ARM64_ATOMIC_RENAME = PASS
ARM64_RELOAD_GENERATION = PASS  # logging level change; generation 1 → 2; GET still 200
ARM64_FD_CLEANUP = PASS
```

Miss path (same binary, real process): sendfile-eligible GET whose `match_sendfile_asset` returns None **hangs** (curl 5s, 0 bytes). Proven for:

- never-existed `/missing.bin`
- `unlink` then GET `/hot.bin`
- path traversal URLs (secret **not** disclosed)
- symlink `escape.link` outside root (secret **not** disclosed)

```text
ARM64_DELETE_RECREATE = FAIL
ARM64_PATH_TRAVERSAL = FAIL  # no secret leak; HTTP reject not delivered (hang)
ARM64_SYMLINK_CONTAINMENT = FAIL  # no secret leak; HTTP reject not delivered (hang)
DEFECT = CAP067_MISS_NEEDHYPER_HANG
  # epoll_worker NeedHyper after match_sendfile_asset None
  # NOT introduced by FD-reuse (never-cached paths hang the same way)
  # FD-reuse design is not reopened here
ORACLE_ARM64_CAP067_REAL_E2E = FAIL_PRODUCT_DEFECT
ORACLE_ARM64_REAL_PRODUCTION = NOT_PROVEN
```

### Superseding repair (separate WIP — FD_REUSE_CAUSAL = NO)

```text
SEE = docs/governance/v044-p1-cap067-miss-needhyper-hang.md
CAP067_MISS_NEEDHYPER_HANG_STATUS = CLOSED_FIXED_AND_VERIFIED_REAL_PRODUCTION
FD_REUSE_CAUSAL = NO
PATTERN = Alternative B (epoll Cap004/miss terminals; NeedHyper = Cap067 decline only)
ORACLE_ARM64_CAP067_REAL_E2E = PASS  # after miss hang repair; dual-arch
ORACLE_ARM64_REAL_PRODUCTION = PASS_FOR_VERIFIED_CAP067_SCOPE
ARM64_DELETE_RECREATE = PASS
ARM64_PATH_TRAVERSAL = PASS
ARM64_SYMLINK_CONTAINMENT = PASS
OPEN_PROVEN_PRODUCT_DEFECTS = 0
V044_P1_FD_REUSE_VERIFICATION_CLOSEOUT_STATUS = owner may reconsider COMPLETE
P2_STARTED = NO
```

Do **not** rewrite R1 FD-reuse history as causal for the hang. R1 hit-path remains
`PROVEN_MATERIAL_SUCCESS`; the hang was an adjacent Cap067 miss/handoff defect.

Original `scripts/reality/cap-067-linux-epoll-sendfile-e2e.py` on this host failed listen: `control socket path is in use by a live listener: /tmp/exyonq.sock` (HOST_SHARED). That is infrastructure for the default socket, not ARM64-incapable hardware.

### Part B — clippy truth

```text
PREVIOUS_CLIPPY_COMMAND = cargo clippy -p exyonq-mod-static --no-deps --color=never -- -D warnings -A dead_code
PREVIOUS_CLIPPY_SCOPE = package exyonq-mod-static --no-deps + -A dead_code
PREVIOUS_DEAD_CODE_ALLOW_REASON =
  ambient unused items in wire_conn.rs / sendfile_fsm.rs (same package)
WIRE_CONN_OWNERSHIP = tracked product in exyonq-mod-static at CURRENT_HEAD
WIRE_CONN_INTRODUCED_BY_FD_REUSE = NO  # last wire_conn commit c5cedd3e (2026-08-19)
AUTHORITATIVE_CLIPPY_COMMAND = cargo clippy --workspace --all-targets -- -D warnings
  # CI also sets RUSTFLAGS=-Dwarnings -Adead_code; --all-features is INVALID
FD_REUSE_AFFECTED_SCOPE_CLIPPY_D_WARNINGS = PASS
  # FD-reuse files clean; package --no-deps with CI -Adead_code exits 0
GLOBAL_CLIPPY_D_WARNINGS = NOT_PROVEN_DUE_TO_PREEXISTING_UNRELATED_STATE
GLOBAL_CLIPPY_NOT_PROVEN_REASON =
  clippy::type_complexity in module-api/src/static_wire.rs:160 (c5cedd3e, not FD-reuse);
  plus ambient dead_code in wire_conn/sendfile_fsm unless RUSTFLAGS -Adead_code
```

CASE CLIPPY-B. No new warning suppressions. No CLIPPY-C FD-reuse warning to fix.

### Optional P3 test

```text
LA-CAP067-FD-001-TEST-001 = PRESERVED_NONBLOCKING
OPTIONAL_TEST_STRENGTHENING = DEFERRED_NOT_JUSTIFIED
  # full wiring needs an invasive open→identity test hook or a flaky race
```
