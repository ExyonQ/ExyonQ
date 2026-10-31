# V044 P1 — CPU efficiency repair Root1

```text
CURRENT_WIP = V044_P1_CPU_EFFICIENCY_REPAIR_ROOT1
ROOT = PIN_STATIC_RUNTIME_PER_EPOLL_WORKER
PRODUCT_MUTATION = YES (this root only)
P1_THROUGHPUT_STATUS = CLOSED_CURRENT_COMPETITIVE_BASELINE
P1_CPU_EFFICIENCY_STATUS = OPEN_REMAINING_ROOTS
P2_STARTED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
```

## Design review (pre-mutation)

```text
DESIGN_PROBLEM =
  Cap067 hot path called runtime() → OnceLock<Mutex<Option<Arc<StaticRuntime>>>> → lock + Arc::clone per request (~1.35% flame self), feeding userspace CPU excess vs NGINX.

EXISTING_PATTERNS_IN_SCOPE =
  Cap067 SessionTable thread_local RefCell; process-lifetime StaticRuntime Arc with in-place bind_roots/bind_compiled_slots (RwLock); pin_runtime at install_kernel_hooks.

RESPONSIBILITIES_BEFORE =
  Publish Arc for epoll FSM; acquire on every match/begin/release/hooks.

OWNERSHIP_BEFORE = process-global Mutex guarding Option<Arc>.
MUTABILITY_BEFORE = Mutex on every hot-path acquire.
CONCURRENCY_MODEL_BEFORE = shared Mutex across all Cap067 workers.
LIFECYCLE_BEFORE = pin once at install; reload mutates same Arc.
GENERATION_INVARIANTS = roots.generation inside StaticRuntime RwLock.
RELOAD_INVARIANTS = bind_* on same Arc; re-pin not required for root freshness.
HOT_PATH_CONSTRAINTS = no Cap004/openat2/registry/WAF/topology change.

PATTERN_CONSIDERED = [worker-local ownership, generation-scoped handle, RAII/guard, direct Arc pinning, NONE]
NO_NEW_PATTERN_ALTERNATIVE = Keep Mutex but return &StaticRuntime — still serializes workers.
PATTERN_SELECTED = worker-local Arc cache + publish epoch (mirrors SessionTable TLS locality)
WHY_SELECTED = Same-Arc reload stays coherent; removes per-request Mutex+clone; epoch covers test/install re-pin.
WHY_ALTERNATIVES_REJECTED = arc_swap = new dep; generation-scoped handle out of Root1; RAII guard adds API churn without win.
SPECULATIVE_ABSTRACTION = FORBIDDEN (not used)
```

## Structural proof

```text
RUNTIME_MUTEX_BEFORE = YES (per request)
RUNTIME_MUTEX_AFTER = YES (publish/cold refresh only)
RUNTIME_ARC_CLONE_PER_REQ_BEFORE = YES
RUNTIME_ARC_CLONE_PER_REQ_AFTER = NO (hot path borrows &StaticRuntime via with_runtime)
SHARED_MUTABLE_STATE_ADDED = NO
ALLOCATIONS_ADDED_PER_REQ = 0
DYN_DISPATCH_ADDED = NO
LOCKS_ADDED = 0 on hot path
LOCKS_REMOVED = per-request Mutex acquire on Cap067 match/begin/release
```

## Product files

- `crates/exyonq-mod-static/src/epoll_session.rs` (only)

## Gates

| Gate | Result |
| --- | --- |
| fmt (`epoll_session.rs`) | PASS |
| ownership_tests (Linux, parallel + serial pin lock) | PASS (4/4) |
| logic audit | PASS (P0/P1=0; P3 nonblocking) |
| architecture-compiler | ACCEPT |
| performance-compiler | ACCEPT (Root1 CPU repair) |
| Cargo.lock | unchanged |
| Flaky `async_shed…` | unrelated; re-run PASS |

## Performance (Netcup amd64 · clean A/B `20260821T005954Z`)

| | BEFORE (reporting) | AFTER Root1 |
| --- | --- | --- |
| Binary SHA-256 | `5a31ccf3…0a13c40` | `14d22eae…57ce91` |
| RPS (median) | 163858.57 | 156952.95 (−4.2%) |
| CPU avg % | 413.08 | 294.02 |
| req/core | 39667.51 | 50618.99 |
| core-ms/req | 0.02521 | 0.01976 (**−21.6%**) |
| P99 | 1.65 ms | **1.64 ms** (30s `--pct` probe) |
| cycles/req | ~76472 | Δ −11684 |

```text
ROOT1_CAUSAL_EFFECT = PROVEN
CPU_COST_PER_REQ_IMPROVEMENT = MEASURED_POSITIVE
RPS_REGRESSION = NO_MATERIAL (−4.2% within historical P1 CV band; throughput baseline not reopened)
P99_REGRESSION = NO_MATERIAL
REAL_PRODUCT_EFFICIENCY_DEFECT = YES
OVERALL_CPU_DEFECT_STATUS = OPEN_REMAINING_ROOTS
  (SendfileHandleRegistry Mutex HashMap; reload::read_state frequency)
PRODUCT_OPTIMIZATION for Root2+ = NOT AUTHORIZED in this WIP
```

## Evidence

`.exyonq-local/benchmarks/` ← sync from Netcup  
`/root/exyonq-v044-p1-auth/.exyonq-local-evidence/v044-p1-cpu-root1-20260821T005954Z/`

Harness: `scripts/remote/v044-p1-cpu-root1-ab.sh`

Causal history preserved in [v044-p1-cpu-efficiency-causal.md](v044-p1-cpu-efficiency-causal.md).
