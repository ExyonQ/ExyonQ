# V044 P1 Cap067 SessionTable lock optimization

**WIP:** `V044_P1_CAP067_SESSIONTABLE_LOCK_OPTIMIZATION`  
**Product commit:** `427b17397c785b0c5105960cd43b604b41ea27d7`  
**Evidence:** `.exyonq-local/benchmarks/v044-p1-sessiontable-lock-20260820T110000Z/`

Preserves absolute-form sendfile fix `f2e90beb` (must not revert).

## Architecture before

Process-global `OnceLock<Mutex<Option<SessionTable>>>` in
`crates/exyonq-mod-static/src/epoll_session.rs`. Every
`begin` / `pump` / `clear` on the Cap067 keepalive/sendfile path took the same mutex.
Under P1 (multiple epoll pool workers), residual profiling measured:

- ~5.45 futex/req
- ~6–8.6% `Mutex::lock_contended` attributed to `epoll_session::sessions`

## Why a global mutex was not required

Each epoll keepalive worker owns its `conns: HashMap<RawFd, ConnState>` and is the only
thread that calls `begin` / `pump` / `clear` for those fds. Sessions do not migrate across
workers; Hyper handoff clears before the fd leaves epoll.

| Question | Answer |
|----------|--------|
| WHY_DOES_SESSIONTABLE_EXIST | Map opaque session id → `SendingState` + access head (module-owned FSM) |
| WHAT_INVARIANT | Session state consistent for one fd’s lifecycle on its owning worker |
| IS_SESSION_OWNERSHIP_ALREADY_SINGLE_REACTOR | YES (per epoll worker) |
| DO_DIFFERENT_SESSIONS_REQUIRE_MUTUAL_EXCLUSION | NO |
| DOES_ONE_SESSION_REQUIRE_MULTI_THREAD_ACCESS | NO |
| IS_GLOBAL_MUTEX_SEMANTICALLY_REQUIRED | NO |

## Selected design

**A — per-worker thread-local SessionTable** (`thread_local! { RefCell<SessionTable> }`).

Not chosen: DashMap / global RwLock / lock striping (would keep shared-table architecture).

Access notify runs **after** releasing the TLS `RefCell` borrow (no re-entrancy).

`StaticRuntime` pin remains a process-global `Mutex` (unchanged; out of session hot map).

## Same-session A/B (Netcup amd64)

Alternating A₁B₁A₂B₂A₃B₃ — identical host/topology/rewrk.

| Metric | A (Mutex) | B (TLS) |
|--------|-----------|---------|
| Median RPS | 67833 | **137977** |
| RPS Δ | — | **+103.4%** |
| futex/req | 5.428 | **1.621** (−70.1%) |
| syscalls/req | 11.467 | **7.695** |
| ctx-switch/req | 0.872 | **0.379** |
| Session `lock_contended` | 8.60% (`epoll_session::sessions`) | **0%** (symbol gone) |
| Remaining `lock_contended` | — | 2.08% (tracing `Stdout::write_all`, not SessionTable) |
| sendfile/req | ≈1.00 | ≈1.00 |
| openat2/req | ≈1.00 | ≈1.00 |

```text
SESSION_MUTEX_CONTENTION_REDUCED = YES
SESSION_MUTEX_CAUSAL_HYPOTHESIS = CONFIRMED
SESSION_MUTEX_COST = REAL_COST_REMOVED
```

## Three-way P1 control (post-fix B)

| Server | Median RPS |
|--------|------------|
| ExyonQ | 143182 |
| NGINX | 164066 |
| OLS | 140570 |

ExyonQ vs NGINX ≈ **−12.7%**; vs OLS ≈ **+1.9%**.

## Dual-Linux

| Host | Result |
|------|--------|
| Netcup amd64 | PASS — unit TLS + real sendfile + A/B + three-way |
| Oracle arm64 | PASS — `session_table_is_thread_local` on matrix tree |

```text
SESSIONTABLE_NEW_PATH_EXECUTED_AMD64 = YES
SESSIONTABLE_NEW_PATH_EXECUTED_ARM64 = YES  # unit ownership path
```

## Decision gates (no further product mutation in this WIP)

| Gate | Verdict |
|------|---------|
| FD_REUSE_REQUIRED | NOT_PROVEN (openat2/req≈1 remains) |
| WORKER_TOPOLOGY_CHANGE_REQUIRED | NOT_PROVEN (still 4 active workers; 4→8 CPU now +11% RPS) |
| WAF_OPTIMIZATION_REQUIRED | NOT_PROVEN |

## Terminal status

```text
P1_STATUS = OPEN_RESIDUAL_GAP_ANALYSIS
RESIDUAL_REPROFILE_REQUIRED = DONE  # post-SessionTable pack below
# Session mutex removed; residual gap still OPEN — do not start next product opt / P2 without owner auth
```

Post-SessionTable residual reprofile (diagnostic, PRODUCT_MUTATION=NO):
`docs/governance/v044-p1-post-sessiontable-residual.md`  
Evidence: `.exyonq-local/benchmarks/v044-p1-post-sessiontable-residual-20260820T132751Z/`

## History preserved

- Pre-fix residual profile: `docs/governance/v044-p1-residual-gap-reprofile.md`
- Absolute-form sendfile: `docs/governance/v044-p1-static-tx-convergence.md` + `f2e90beb`
- Post-SessionTable residual: `docs/governance/v044-p1-post-sessiontable-residual.md`
