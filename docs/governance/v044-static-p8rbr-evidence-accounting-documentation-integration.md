# V044 — P8R-BR evidence accounting + documentation candidate (P8RBRDOC-A)

```text
WIP = V044_STATIC_P8RBR_EVIDENCE_ACCOUNTING_AND_DOCUMENTATION_INTEGRATION
CASE = P8RBRDOC-A
STATUS = P8R_BR_EVIDENCE_ACCOUNTING_RECONCILED_AND_DOCUMENTATION_CANDIDATE_SEALED
MODE = EVIDENCE_ACCOUNTING_AND_DOCUMENTATION_ONLY
OWNER_AUTHORIZATION = YES_DOCS_AND_EVIDENCE_ONLY
```

## Owner admission (qualified)

```text
CURRENT_PRODUCT_TERMINAL = P8R-B
CURRENT_ADMITTED_REORACLE_TERMINAL = P8R-BR
P8R_BR_OWNER_ADMISSION = ACCEPT_QUALIFIED
P8R_A_CLAIMED = NO
PRODUCT_REPAIR_REQUIRED = NO
SOURCE_PROVENANCE_MODE = EXACT_BINARY_REPLAY
CLEAN_GIT_REPRODUCIBILITY = NO
WORKSPACE_GATES = NOT_RUN_INCOMPLETE_ATTRIBUTABLE_SOURCE
```

This WIP does **not** mutate product, harness semantics, tests, dependencies, or the dirty canonical worktree. It reconciles authoritative evidence `20260902T080000Z` / `20260902T080000Z-arm64`, corrects unsupported resource accounting from detached draft `573a89fb`, and seals a **docs-only candidate branch**.

## Why “no further action” was rejected

1. Documentation lived only on detached commit `573a89fb` (not on a durable candidate branch from canonical HEAD).
2. Resource series claims were incompatible with raw `RESOURCES/series.txt` (FD labeled as RSS).
3. `DOC_MUTATION=YES_REQUIRED` was unsatisfied for canonical documentation integration readiness.

## Authority

| Item | Value |
|------|-------|
| Canonical HEAD (unchanged) | `48674836861851217c968720a00e18b03b693b51` |
| Canonical TREE | `65ec6aade63f60e32f3ee9927ef8e7229015bcca` |
| Authoritative AMD64 | `20260902T080000Z` |
| Authoritative ARM64 | `20260902T080000Z-arm64` |
| Invalid run | `20260902T070000Z` (excluded) |
| Evidence inventory SHA256 | `73ac7af8f407bfcbd999482dcf78d75eecded181a1b12fcc45156a7088f32c21` |
| Artifact count | 37 |

Narrative + claim tables: [v044-static-eagain-partial-sendfile-toctou-reoracle.md](v044-static-eagain-partial-sendfile-toctou-reoracle.md).
Machine terminal: [v044-static-p8rbr-evidence-accounting-documentation-integration-terminal-block.txt](v044-static-p8rbr-evidence-accounting-documentation-integration-terminal-block.txt).

## Resource accounting correction

| Prior claim (573a89fb) | Raw evidence | Corrected |
|------------------------|--------------|-----------|
| `FD_SERIES_AMD64=3440,3540,…` | `RESOURCES/series.txt` `FD=12` | `12,12,12,12,12,12,12,12` |
| (no ARM64 FD series) | ARM64 `FD=12` | `12,12,12,12,12,12,12,12` |
| `THREAD_SERIES=NOT_CAPTURED` + `THREAD_STAIRCASE=0` | `THREADS=8` constant | series captured; staircase `0` = no growth |
| `CHILD_STAIRCASE=0` / `SOCKET_STAIRCASE=0` | no samples | `NOT_CAPTURED` / `NOT_PROVEN` |

```text
RESOURCE_SERIES_ACCOUNTING = RECONCILED_WITH_RAW_EVIDENCE
UNSUPPORTED_ZERO_STAIRCASE_CLAIMS_REMOVED = YES
NO_OBSERVATION != PROOF_OF_ABSENCE
```

## Candidate branch

```text
CANDIDATE_BRANCH = local/p8rbr-evidence-accounting-documentation-integration
BASE_COMMIT = 48674836861851217c968720a00e18b03b693b51
CANONICAL_MERGE = NO
```

## Explicit non-claims

```text
P8R_A_CLAIMED = NO
ANTI_KOBAYASHI = NOT_MEASURED
TOCTOU_HASH_MISMATCHES = NOT_MEASURED
CONFIG_DISCLOSURE = NOT_MEASURED
SYMLINK_TOCTOU_STATUS = BOUNDED_RESIDUAL
PHASE8_REMAINDER_AUTHORIZED = NO
PHASE_9_AUTHORIZED = NO
```
