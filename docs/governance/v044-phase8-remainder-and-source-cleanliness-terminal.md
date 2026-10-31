# V044 Phase 8 remainder and source-cleanliness terminal

```text
WIP = V044_PHASE8_REMAINDER_AND_SOURCE_CLEANLINESS_TERMINAL
CASE = P8REM-D
STATUS = BLOCKED_WORKSPACE_CLIPPY_ALL_FEATURES_AND_PHASE8_ORACLE_INCOMPLETE
PARENT = P8RBRDOCINT-A / P8R-BR / P8R-B
```

Supersession (identity / P3 / Anti-Kobayashi / worktree cleanliness only): see
[v044-p8rem-candidate-consistency-and-blocker-attribution.md](v044-p8rem-candidate-consistency-and-blocker-attribution.md)
(**CASE P8RECON-A**). Historical `P8REM-D` remains; Phase 8 still open. Do not use
`3d692422` as final candidate HEAD — reconciled tip before reconciliation docs was `3f0d1c40`.

## Entry

Canonical `release/v0.4.4-integration` at `cd14277e` with ambient fingerprints preserved.
Isolated candidate worktree only; canonical dirty worktree not mutated.

## Doc hygiene

Two trailing-whitespace lines repaired (`DOC_DIFF_CHECK=PASS`). Not previously counted in `OPEN_LOGIC_P3=3`.

## Source

Ambient inventory classified; one credential-bearing remote harness excluded.
Attributable CFD source + harness assembled. P8R-BR 33-file manifest fully present.
Clean dual-Linux native builds reproduce historical P8R-BR binaries exactly.

## Gates

| Gate | Result |
| --- | --- |
| fmt --check | PASS |
| check --workspace --all-targets --locked | PASS |
| clippy --workspace --all-targets --all-features --locked -D warnings | FAIL (preexisting on entry HEAD control) |
| test --workspace --all-targets --locked (Darwin) | FAIL (`foundation_matrix` under suite contention) |
| foundation_matrix Linux amd64/arm64 | PASS 14/14 |
| project/benchmark integrity gates | PASS |
| no-smoke selftest | PASS |

## Phase 8 dual-Linux

Runs `20260902T152000Z-amd64-p8` / `…-arm64-p8` on clean binaries.
Harness patched for dual-arch + `CARGO_TARGET_DIR` (prior runs invalidated).
`CORE_MATRIX=FAIL_failures_2`; `GATE_PHASE6B_FCGI=FAIL` (`trailing_data` Linux intermittent 11/20 — not proven product defect); `STATIC_PREAD_FALLBACK=NOT_MEASURED`; `STATIC_EAGAIN=INCOMPLETE`; `ANTI_KOBAYASHI` harness-declared PASS — **reclassified** `NOT_PROVEN_FROM_AVAILABLE_EVIDENCE` under P8RECON-A.


## Logic audit

Independent `exyonq-logic-auditor` on candidate tip `3d692422` / tree `deae5f10`:

```text
LOGIC_AUDIT_STATUS = PASS_WITH_NONBLOCKING_P3_FINDINGS
OPEN_LOGIC_P0 = 0
OPEN_LOGIC_P1 = 0
OPEN_LOGIC_P2 = 0
OPEN_LOGIC_P3 = 5
FINDINGS = P8REM-LA-001 .. P8REM-LA-005
PRODUCT_DEFECT_FOUND = NO
SUPPORTED_CONTRACT_MUST_FIX_COUNT = 0
RECOMMENDED_CASE = P8REM-D
REJECT_P8REM_A = YES
REJECT_P8REM_B = YES
REJECT_P8REM_C = YES
```

`FINAL_OPEN_LOGIC_P3=3` = parent residuals `P8RBR-P3-001..003` (TOCTOU hash / config disclosure / Anti-K not measured). The five `P8REM-LA-001..005` are additive. Unique open P3 after reconciliation = **8** (`LOGIC_P3_ACCOUNTING=RECONCILED_BY_ID`). Doc whitespace was **not** in the parent-3 set. Does not upgrade CASE to C.

## Product repair

```text
PRODUCT_REPAIR_ATTEMPTED = NO
SUPPORTED_PRODUCT_DEFECT_REPAIR = NOT_AUTHORIZED
```

## Next

```text
NEXT_RECOMMENDED_WIP = V044_PHASE8_CLIPPY_ALL_FEATURES_AND_ORACLE_COMPLETION
NEXT_WIP_AUTHORIZED = NO
PHASE_9_AUTHORIZED = NO
```
