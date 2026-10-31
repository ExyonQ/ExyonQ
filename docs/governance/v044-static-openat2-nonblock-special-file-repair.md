# V044 — Static openat2 nonblock special-file repair

```text
WIP = V044_STATIC_OPENAT2_NONBLOCK_SPECIAL_FILE_REPAIR
PARENT_TERMINAL = P8-C
PRIMARY_DEFECT = P8-STATIC-FIFO-001
CURRENT_ADMITTED_TERMINAL = P8R-B
CASE = P8R-B
STATUS = FIFO_DEFECT_CLOSED_WITH_EXPLICIT_NONBLOCKING_QUALIFICATIONS
OWNER_ADMISSION = ACCEPT
REPORTED_TERMINAL_COMMIT_FULL = f087a4ea971ee1cde48c0d40960404827fe55813
REPORTED_TERMINAL_TREE = 4fd9174d4e25cb83781913788738a710918625fa
PRODUCT_REPAIR_AUTHORIZED = YES_FOR_P8_STATIC_FIFO_001_ONLY
PRODUCT_REPAIR_PERFORMED = YES
PHASE8_REMAINDER_AUTHORIZED = NO
PHASE_9_AUTHORIZED = NO
```

Repairs **only** `P8-STATIC-FIFO-001`. Does **not** close Phase 8 remainder, Phase 9, or dual-Linux Static qualification in full (EAGAIN/partial sendfile/oracles deferred).

## Root cause

Linux `openat2_confined` opened with `O_RDONLY|O_CLOEXEC` only. FIFO `openat2` blocked in `wait_for_partner` before `classify_opened` (`REGULAR_FILES_ONLY`). Canonicalize fallback already used `O_NONBLOCK`.

## Selected architecture

**Alternative A** — `O_NONBLOCK` on primary `openat2` + existing `classify_opened` on opened FD. See [v044-static-openat2-nonblock-special-file-repair-architecture.md](v044-static-openat2-nonblock-special-file-repair-architecture.md).

```text
SELECTED_ARCHITECTURE = Alternative A
OBJECT_IDENTITY_GUARANTEE = open FD → fstat on same FD
TOCTOU_MODEL = unchanged BOUNDED_RESIDUAL (not re-run this WIP)
CLIENT_VISIBLE_FAILURE = FIFO/unix/special → 403 Forbidden
```

## Product change

`crates/exyonq-cfd-dataplane/src/static_serve.rs`:

- `how.flags` includes `O_NONBLOCK` on Linux `openat2`
- `map_open_err`: `ENXIO` → `StaticErr::Forbidden` (defensive mapping only; no-writer FIFO causal path is `FIFO_PATH_A`, not ENXIO)

## Evidence

| Run | Platform | Result |
| --- | --- | --- |
| `20260901T204300Z` | Netcup amd64 | FIFO 403 ~7ms; hello after fifo 200; 4-shard concurrent PASS |
| `20260901T204500Z-arm64` | Oracle arm64 native | Same PASS pattern |

```text
AMD64_BINARY_SHA256 = eee3e4d93dd565988afc61be7cf731cc8a9c94f79c999eae2aec0f5a75fcaffe
ARM64_BINARY_SHA256 = 72652758ade208fb4d1685dd498077fd1c0d787370bfab9e6fb41bc019bfe918
P8_PRE_BINARY_SHA256 = 6d9bc14db019633ecbd3519ea8cb8e78ce11a76abaedb0d039f95bb9ad8c413e
static_serve.rs SHA256 = dc7965de88b18ba9981babb6cecb9d27e2ecd005e2b72a51cf83dd28f3b4db2e
```

PRE reproduction: Phase 8 `20260901T181000Z` kernel stacks (not re-built pre-repair binary this WIP).

## Explicit nonblocking qualifications (P8R-B)

| Item | Status |
| --- | --- |
| EAGAIN full payload SHA256 | **NOT_REMEASURED_THIS_WIP** |
| Partial sendfile full SHA256 | **NOT_REMEASURED_THIS_WIP** |
| TOCTOU 4000 relative race | **NOT_REMEASURED_THIS_WIP** |
| `STATIC_SENDFILE` strace REAL_LINUX | **NOT_MEASURED_THIS_WIP** (hash regression PASS only) |
| `OPENAT2_ONONBLOCK_FLAG` strace sidecar | **REVIEW** (source + runtime liveness; flag grep inconclusive) |

## Owner terminal requalification (20260902)

Owner retained initial **P8R-B** as not automatically admitted, then authorized
`V044_STATIC_OPENAT2_REPAIR_TERMINAL_REQUALIFICATION` (evidence-only, no product mutation).

```text
P8R-B_REPORTED = VALID_HISTORICAL_SUBMITTED_RESULT_NOT_AUTOMATICALLY_ADMITTED
REQUALIFICATION = LATER_OWNER_ADMISSION_EVIDENCE
COMMIT_CHAIN = PASS (d90c8866→02abdace→3a6d01ba→d47ffe10→f087a4ea)
FIFO_CAUSAL_SYSCALL_PATH = FIFO_PATH_A (openat2 O_NONBLOCK succeeds → fstat → !is_file → 403)
ENXIO_OBSERVED_IN = NONE for no-writer FIFO (defensive branch only)
OWNER_ADMISSION = ACCEPT for P8R-B bounded FIFO closure
P8R-A = NOT_SUPPORTED (partial requal; TOCTOU/sendfile/EAGAIN not executed)
```

Evidence: `.exyonq-local/evidence/static-openat2-repair-terminal-requalification/20260902T010800Z/` (amd64 FIFO subset valid) and `...-arm64/` (same). Invalid harness runs: `20260901T220000Z`–`20260901T231000Z`; `20260902T010800Z` = partial campaign (FIFO core PASS; connect-disconnect `NOT_MEASURED` per `REQUAL-HARNESS-001`) — see requal terminal block.

## P8-C history

```text
P8-C = HISTORICAL_VALID_DEFECT_TERMINAL (unchanged)
P8-STATIC-FIFO-001 = PROVEN_AT_P8-C; CLOSED_BY_THIS_WIP on dual Linux FIFO liveness oracle
```

## Worktree note

At repair entry, git HEAD matched `d948566…` but the worktree had extensive ambient dirty/untracked CFD state beyond P8-C docs. Repair touched **only** `static_serve.rs`, bounded tests (unchanged pass), harness scripts, and governance docs.

## Specialists (post-repair)

Post-requal documentation seal: logic-auditor **PASS_WITH_NONBLOCKING_P3_FINDINGS** (`OPEN_LOGIC_P3 = 4` harness/doc scope; `OPEN_PRODUCT_P3 = 0`). GRC **CONDITIONAL_ADMIT_P8R-B_NOT_P8R-A**. Pre-mutation: Vertex **CONDITIONAL_ACCEPT** on Alt A; Security **REJECT pre / repair via A**.

## Later reoracle admission (P8R-BR) — additive, does not rewrite P8R-B

Owner-qualified admission of deferred Static runtime oracles under MODE A exact binary replay:

```text
CURRENT_PRODUCT_TERMINAL = P8R-B (unchanged)
CURRENT_ADMITTED_REORACLE_TERMINAL = P8R-BR
P8R_BR_OWNER_ADMISSION = ACCEPT_QUALIFIED
P8R_A_CLAIMED = NO
AUTHORITATIVE_RUNS = 20260902T080000Z + 20260902T080000Z-arm64
CLEAN_GIT_REPRODUCIBILITY = NO
```

Narrative: [v044-static-eagain-partial-sendfile-toctou-reoracle.md](v044-static-eagain-partial-sendfile-toctou-reoracle.md).
Evidence-accounting documentation candidate: [v044-static-p8rbr-evidence-accounting-documentation-integration.md](v044-static-p8rbr-evidence-accounting-documentation-integration.md).

FIFO causal path and P8R-B qualifications remain authoritative for product FIFO closure. P8R-BR does not reopen `P8-STATIC-FIFO-001`.

## Not authorized

Phase 8 remainder, Phase 9, merge, push, tag, release, publication.
