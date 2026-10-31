# V044 Phase 8 FastCGI `trailing_data` causal closure

```text
WIP = V044_PHASE8_FCGI_TRAILING_DATA_CAUSAL_CLOSURE
CASE = FCGITD-A
STATUS = FCGI_TRAILING_DATA_HARNESS_CAUSALITY_CLOSED
PARENT_CASE = P8TECH-D
PHASE_8_CLOSED = NO
STATIC_PREAD_FALLBACK_RUNTIME = NOT_MEASURED
```

## Root cause

```text
ROOT_CAUSE = HARNESS_RACE_PROVEN
PRODUCT_DEFECT_PROVEN = NO
```

`trailing_data_forces_discard_error` used a TCP scripted peer that `write_all`’d `END_REQUEST` and `TRAIL` in **separate** syscalls. Product SG-FCGI-05 only rejects **leftover bytes already in the read buffer** after parsing `END_REQUEST` (unit Cursor oracle). When `TRAIL` arrived in a later segment after completion, the exchange returned `Ok` — intermittent on Darwin/Linux.

## Causal evidence

| Variant | Result (20 reps) |
| --- | --- |
| A cobuffered END+TRAIL one write | TrailingData 20/20 (Darwin + amd64 + arm64) |
| B2 END then 50ms then TRAIL | Ok 20/20 (late not required at completion) |
| F clean | Ok 20/20 |
| Baseline separate writes (pre-fix) | Darwin 1/20 Ok; parent Netcup 12/20, Oracle 9/20 FAIL |

## Repair

Harness/test only (`bb15757f` + rustfmt `409b02be`):

- Co-buffer `END_REQUEST` + `TRAIL` in `phase6b_fcgi` peer.
- Add `trailing_data_causal_matrix` characterization tests.

```text
PRODUCT_FILES_CHANGED = 0
PRODUCT_BEHAVIOR_CHANGE = NO
ORACLE_STRENGTH_PRESERVED = YES
```

## Requalification

| Gate | Darwin | Netcup | Oracle |
| --- | --- | --- | --- |
| trailing_data 20-rep | 0 fail | 0 fail | 0 fail |
| FCGI pool 20-rep | 0 fail | 0 fail | 0 fail |
| Phase 8 `GATE_PHASE6B_FCGI` | — | PASS | PASS |
| Phase 8 `CORE_MATRIX` | — | PASS | PASS |
| CFD binary SHA | — | `eee3e4d9…` | `72652758…` (P8R-BR match) |

## Explicit non-claims

```text
PHASE_8_CLOSED = NO
PREAD = NOT_MEASURED
PHASE_9 = NOT_AUTHORIZED
CANONICAL_MERGE = NO
```

P8TECH-D remains a valid historical terminal. This WIP closes only the trailing_data harness race.
