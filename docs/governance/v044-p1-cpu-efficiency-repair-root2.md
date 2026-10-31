# V044 P1 — CPU efficiency repair Root2

```text
CURRENT_WIP = V044_P1_CPU_EFFICIENCY_REPAIR_ROOT2
ROOT = BYPASS_SENDFILE_HANDLE_REGISTRY_ON_IN_PROCESS_EPOLL_PATH
PRODUCT_MUTATION = TRIED_THEN_REVERTED
ROOT1_STATUS = CLOSED_PROVEN (preserved)
P1_THROUGHPUT_STATUS = CLOSED_CURRENT_COMPETITIVE_BASELINE
P1_CPU_EFFICIENCY_STATUS = OPEN_REMAINING_ROOTS
P2_STARTED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
```

## Design review (pre-mutation)

```text
DESIGN_PROBLEM =
  Cap067 match→issue(SendfileHandleRegistry Mutex<HashMap>)→opaque handle→begin(take)
  costs ~0.99% flame self while the asset is already owned in-process on the same worker.

EXISTING_PATTERNS_IN_SCOPE =
  Cap067 SessionTable TLS; Root1 TLS StaticRuntime pin; KD2.2 registry for Hyper/dispatch.

RESPONSIBILITIES_BEFORE =
  Registry holds Arc<SendfileAsset> between match and begin/release across module-api hooks.

OWNERSHIP_BEFORE = global Mutex<HashMap<u64, Entry>> on StaticRuntime.
LIFETIME_BEFORE = issue → take-once or release; generation invalidate on bind_roots (Hyper).
HANDLE_LIFECYCLE = match issues; begin takes; abort releases (LA-CAP067-001).
REGISTRY_WRITERS = issue / invalidate_through_generation
REGISTRY_READERS = take / release / active_count
CROSS_THREAD_USAGE = Mutex shared across Cap067 workers (contention).
CROSS_PROCESS_USAGE = NO
RELOAD_GENERATION_DEPENDENCY = registry tags (Hyper); Cap067 also checks attachment generation before match.
HOT_PATH_FREQUENCY_PER_REQUEST = issue + take ≈ 2 Mutex acquires + HashMap insert/remove.

IN_PROCESS_EPOLL_REQUIRES_GLOBAL_HANDLE_REGISTRY = NO
  (same-thread Cap067; hooks can stay SendfileHandle-shaped via TLS pending + sentinel 0;
   Hyper still needs the registry.)

PATTERN_CONSIDERED =
  [direct Arc ownership, worker-local ownership, typed handle, registry specialization,
   existing registry unchanged, NONE]
PATTERN_SELECTED = worker-local TLS pending Arc + SendfileHandle(0) sentinel
WHY_SELECTED = Removes Cap067 Mutex/HashMap without widening module-api / core; Hyper unchanged.
WHY_ALTERNATIVES_REJECTED =
  Arc in hooks → REJECT boundary; remove registry globally → breaks Hyper tests/materialize;
  leave unchanged → no Root2 experiment.
```

## Experiment (Netcup amd64)

```text
BEFORE = Root1 terminal binary 14d22eae…57ce91
AFTER  = Root1+Root2 binary 1be45f84…01b5012
Evidence = .exyonq-local-evidence/v044-p1-cpu-root2-20260821T013802Z/
Harness = scripts/remote/v044-p1-cpu-root2-ab.sh
SENDFILE_RUNTIME_EXECUTED = YES (~145k sendfile in ~8s strace sample under load)
```

| Metric | BEFORE Root2 (Root1) | AFTER Root2 | Δ |
| --- | --- | --- | --- |
| core-ms/req | 0.01976 | 0.019694 | **−0.335%** |
| req/core | 50618.99 | 50777.46 | +0.31% |
| CPU avg % | 294.02 | 292.92 | −0.37% |
| RPS median | 156952.95 | 151178.99 | −3.68% (noise class) |
| cycles/req | (Root1 ~64.8k class) | 62275 | — |
| IPC | — | 1.11 | — |

P99 table from `--pct` did not land in harness JSON (rewrk 0.3.1 `--json` omits percentiles); not used to claim PROVEN.

Aggregate vs original pre-repair:

```text
ORIGINAL_PRE_REPAIR_CORE_MS_PER_REQ = 0.02521
AFTER_ROOT1_CORE_MS_PER_REQ = 0.01976
AFTER_ROOT2_CORE_MS_PER_REQ = 0.019694
ROOT2_INCREMENTAL_DELTA_PERCENT = -0.335
ROOT1_PLUS_ROOT2_CUMULATIVE_DELTA_PERCENT ≈ -21.88  # vs original; incremental Root2 ≈ noise
```

## Verdict

```text
ROOT2_CAUSAL_EFFECT = FALSIFIED_OR_TOO_SMALL
PERFORMANCE_VALUE = NOT_PROVEN
REFACTORING_VALUE = NOT_KEPT  # dual ownership not justified without CPU win
PRODUCT_MUTATION = REVERTED
PERFORMANCE_COMPILER = REJECT_PERFORMANCE
ARCHITECTURE_COMPILER = ACCEPT (on trial design; mutation not retained)
LOGIC_AUDIT = PASS (P0/P1=0 on trial HEAD; recheck N/A after revert)
```

Root2 is **not** CLOSED_PROVEN. Product tree restored to Root1 Cap067 registry path + Root1 TLS pin. Netcup runtime binary restored to Root1 SHA `14d22eae…`.

## Remaining

```text
OVERALL_CPU_DEFECT_STATUS = OPEN_REMAINING_ROOTS
REMAINING_ROOT = reload::read_state frequency
FD_REUSE = OPTIONAL_FUTURE_OLS_DIFFERENTIAL_NOT_AUTHORIZED
P2_STARTED = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
STOP
```
