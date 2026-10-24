# V044 Phase 8 `pread` fallback reality admission — entry abort

```text
WIP = V044_PHASE8_PREAD_FALLBACK_REALITY_ADMISSION_OR_EXPLICIT_BLOCKER
CASE = PREAD-D
STATUS = ABORTED_ENTRY_IDENTITY_OR_CLEANLINESS_MISMATCH
PARENT_CASE = FCGITD-A
OWNER_AUTHORIZATION = YES_BOUNDED_PREAD_REALITY_ADMISSION
PHASE_8_CLOSED = NO
PHASE8_CANDIDATE_QUALIFIED = NO
```

## Abort reason

Prompt required candidate HEAD exactly:

```text
82ac7969360f08a5712e02c65e53fb64afe4cc86
```

Observed on `local/phase8-remainder-source-cleanliness`:

| Field | Value |
|-------|-------|
| Actual HEAD | `82ac4ec2c404158aa5c369811d601636c1cb6094` |
| Actual tree | `d956cda3175d424f775e443a8852e7da3c2caeb1` |
| Clean | YES |
| Expected object in repo | **NO** (`fatal: bad object`) |

Hamming distance vs expected full SHA = 35 (not a truncated/prefix typo of the FCGITD tip). Actual tip is the sealed **FCGITD-A** docs bind commit (`docs(governance): record FCGITD-A authoritative tip pointers`).

Canonical fingerprints matched freeze (`cd14277e` + ambient 211). Canonical was not mutated.

## Work not started

Per fail-closed entry gate: no harness/product mutation, no dual-Linux pread matrix, no natural-trigger probes beyond entry.

## Owner options

1. Re-authorize entry on actual tip `82ac4ec2…` (FCGITD-A seal) and re-issue this WIP.
2. Provide the correct existing commit object that should be the pread entry tip.
3. If `82ac7969…` was meant to exist on another machine/worktree, sync that object into this candidate before retry.

```text
NEXT_WIP_AUTHORIZED = NO
CANONICAL_MERGE = NO
PHASE_9_AUTHORIZED = NO
STOP = YES
```
