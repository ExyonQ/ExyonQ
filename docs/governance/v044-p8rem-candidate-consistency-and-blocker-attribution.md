# V044 P8REM candidate consistency and blocker attribution

```text
WIP = V044_P8REM_CANDIDATE_CONSISTENCY_AND_BLOCKER_ATTRIBUTION
CASE = P8RECON-A
STATUS = P8REM_CANDIDATE_RECONCILED_BLOCKERS_ATTRIBUTED_PHASE8_STILL_OPEN
PARENT_CASE = P8REM-D
OWNER_AUTHORIZATION = YES_READ_ONLY_PLUS_CANDIDATE_HYGIENE
```

## Scope

Read-only reconciliation of the isolated `local/phase8-remainder-source-cleanliness` candidate plus worktree hygiene and documentation. **No** product/harness/test repair, dual-Linux rerun, Phase 8 close, or canonical merge.

## Candidate identity

| Field | Value |
| --- | --- |
| Reported historical tip | `3d692422…` (finalize P8REM-D bind) |
| Logic-audit bind | `3f0d1c40d30778d9b7119f3b3d2879280df6996e` |
| Reconciled entry HEAD (pre-this-WIP) | `3f0d1c40…` / tree `f5c1869f…` |
| Commit count from `cd14277e` | 9 (before this WIP's doc commits) |
| Chain | linear descendant; `3d692422` is parent of `3f0d1c40` |

Do not treat `3d692422` as `CANDIDATE_FINAL_HEAD`.

## Worktree hygiene

Before: three unstaged tracked paths (competitive dashboard HTML/JSON + generator script).

- Preserved under evidence `CANDIDATE_DIRT/preserved/`.
- Restored **explicit paths only** to candidate `HEAD` (`git restore --source=HEAD -- <paths>`).
- After: `git status --porcelain` empty → `CANDIDATE_WORKTREE_CLEAN=YES`.
- Secret path `scripts/remote/v044-split-plane-near-product-proving-ground.sh`: excluded; not read, copied, or committed.

## Logic P3 accounting (by ID)

Parent `FINAL_OPEN_LOGIC_P3=3` maps to residual **not-measured** fields from P8RBRDOCINT/P8R-BR (no separate LA IDs in that seal):

| ID | Description |
| --- | --- |
| P8RBR-P3-001 | `TOCTOU_HASH_MISMATCHES=NOT_MEASURED` |
| P8RBR-P3-002 | `CONFIG_DISCLOSURE=NOT_MEASURED` |
| P8RBR-P3-003 | `ANTI_KOBAYASHI=NOT_MEASURED` (parent) |

P8REM logic auditor (`05a75010…`) added five new IDs `P8REM-LA-001`…`005`. **Zero duplicates** with parent IDs. **Zero closed.**

```text
PARENT_OPEN_LOGIC_P3 = 3
P8REM_NEW_LOGIC_P3 = 5
P8REM_DUPLICATED_LOGIC_P3 = 0
CURRENT_UNIQUE_OPEN_LOGIC_P3 = 8
OPEN_PRODUCT_P3 = 0
SUPPORTED_CONTRACT_MUST_FIX_COUNT = 0
LOGIC_P3_ACCOUNTING = RECONCILED_BY_ID
```

Doc trailing-whitespace repair remains **not** part of the parent-3 set.

## Anti-Kobayashi

Harness declares `ANTI_KOBAYASHI=PASS` via source grep for run marks / `HARDCODED_PHASE8_PASS` (lines ~1233–1244). No documented negative control, expected/observed adversarial flip log, or external side-effect oracle in the available evidence package.

```text
ANTI_KOBAYASHI = NOT_PROVEN_FROM_AVAILABLE_EVIDENCE
```

`PASS_HARNESS_DECLARED` is rejected as admission.

## Blocker attribution (no repair)

| Blocker | Attribution |
| --- | --- |
| Clippy `--all-features` | `PREEXISTING_WORKSPACE_GATE_FAILURE` on entry `cd14277e`; 9× `dead_code` in `streaming_wire_writer.rs`; **blocks Phase 8 gate** |
| Darwin FCGI pool 2/20 | Intermittent Dirty≠Clean; product defect **not proven** |
| Linux `trailing_data` 11/20 | Intermittent; product defect **not proven**; drives `GATE_PHASE6B_FCGI=FAIL` |
| `CORE_MATRIX` | Failures include `FASTCGI_SCRIPT_MAPPING_CONFUSION=1` (likely harness); product defect **not proven** |
| EAGAIN / partial sendfile on Phase 8 runs | Same binary SHA as P8R-BR; **different harness** (`eacb1393…` vs historical `ba4e79db…`) → attribution **HARNESS**; historical P8R-BR MODE A remains valid; generalization to Phase 8 remainder **NO** |
| `pread` fallback | `NOT_MEASURED` (sendfile success path; oracle not executed) |

```text
PRODUCT_DEFECT_PROVEN = NO
PRODUCT_REPAIR_ATTEMPTED = NO
PHASE_8_CLOSED = NO
```

## Evidence

```text
.exyonq-local/evidence/p8rem-candidate-consistency-blocker-attribution/20260902T185701Z/
```

## Follow-on

Owner-authorized WIP `V044_PHASE8_CLIPPY_ALL_FEATURES_AND_ORACLE_COMPLETION` sealed as **CASE P8TECH-D** — see [v044-phase8-clippy-all-features-and-oracle-completion.md](v044-phase8-clippy-all-features-and-oracle-completion.md). Clippy `--all-features`, EAGAIN/partial (P8R-BR oracle), SCRIPT_MAPPING, and Anti-Kobayashi negative controls advanced; pread + intermittent `trailing_data` still block Phase 8 close. Phase 9 / competitive benches remain forbidden.

Parent `P8REM-D` history is preserved; this document supersedes only identity/P3/Anti-K/worktree cleanliness fields for the candidate.
