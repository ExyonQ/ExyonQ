# V044 P1 — CPU efficiency repair Root3 (sealed)

```text
CURRENT_WIP = V044_P1_CPU_EFFICIENCY_REPAIR_ROOT3
ROOT = REDUCE_RELOAD_READ_STATE_FREQUENCY_ON_CAP067_P1_SENDFILE_HOT_PATH
PRODUCT_MUTATION = YES (KEPT / SEALED)
ROOT1_STATUS = CLOSED_PROVEN
ROOT2_STATUS = CLOSED_FALSIFIED_REVERTED
ROOT3_STATUS = CLOSED_PROVEN
P1_THROUGHPUT_STATUS = CLOSED_CURRENT_COMPETITIVE_BASELINE
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
```

## Commits

```text
ROOT1_COMMIT = e07817526035cb12b4594d46848ef1a60e5bc3e5
CPU_CAUSAL_DOCS_COMMIT = cd2803d2f685e3b2e7ae554715aedb0cacd1fc27
ROOT3_PRODUCT_COMMIT = 2462410ec93fc54d50e6ef1f977d71e17f370450
ROOT3_PRODUCT_TREE = 02fe67194b3f850266cbd4abd2f157ecf761dcff
```

## Design

```text
PER_REQUEST_READ_STATE_REQUIRED_FOR_CORRECTNESS = NO
PATTERN_SELECTED = AtomicU64 active_runtime_generation + published WAF wire-inspection AtomicBool
READ_STATE_CALLS_PER_REQ_BEFORE = 2
READ_STATE_CALLS_PER_REQ_AFTER = 0   # Cap067 P1 WAF-off steady state
```

Fail-closed:

1. WAF-on published before `shared.write` on enable (LA-CAP067-R3-001)
2. Early-exit re-samples published flag after `reload_in_progress` (LA-CAP067-R3-002)

## Performance (Netcup A/B vs Root1)

| | BEFORE Root1 | AFTER Root3 |
|--|-------------|-------------|
| Binary (trial) | `14d22eae…` | `46e79a88…` (pre-TOCTOU re-sample) |
| core-ms/req | 0.01976 | **0.018077 (−8.51%)** |
| RPS | 156952.95 | 161955.19 (+3.19%) |
| P99 | 1.64 ms | 1.52 ms |

```text
ROOT3_CAUSAL_EFFECT = PROVEN
CUMULATIVE_CPU_COST_DELTA_FROM_ORIGINAL ≈ -28.29%
PERFORMANCE_COMPILER = ACCEPT
ARCHITECTURE_COMPILER = ACCEPT
LOGIC_AUDIT = PASS (P0/P1=0)
```

Sealed product includes the R3-002 re-sample; live three-way uses the rebuilt sealed binary.

## Product files

- `core/src/reload/mod.rs`
- `core/src/server/connection_executor.rs`
- `core/src/kernel/epoll_attach.rs`

Harness: `scripts/remote/v044-p1-cpu-root3-ab.sh`  
Live three-way: `scripts/remote/v044-p1-cpu-live-3way.sh` / `docs/governance/v044-p1-cpu-efficiency-live-3way.md`
