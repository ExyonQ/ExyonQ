# ExyonQ — Roadmap (canonical)

```text
DOCUMENT = ROADMAP.md
DOCUMENT_ROLE = CANONICAL_GLOBAL_ROADMAP
AUTHORITY = SOLE for R* global phase identifiers
LAST_UPDATED = 2026-08-04
BRANCH = main
PUBLIC_RELEASE = v0.4.4.1
FINAL_PUBLISHED_RELEASE_HEAD = 99bd7c547cb4d7c7821a1e131dfca5cf8d8db987
R3_PRODUCT_BASELINE = f0b2d67
R3_CLOSE_DOCUMENTATION_TIP = d0814bc79ea08b91b753269e0883ab249bf7c3b9
R5_STATUS = COMPLETE
R5_RESULT = PASS
R5_CLOSED = YES
R6_STATUS = NOT_STARTED
```

Status companion: [`PROJECT_STATUS.md`](PROJECT_STATUS.md)  
Documentation map: [`DOCUMENTATION_INDEX.md`](DOCUMENTATION_INDEX.md)

```text
Only R* determines the current global project phase.
```

Internal tracks (BV04-P\*, K8S-P\*, P14\*, K\*/KD\*, MR\*, Cursor plans) do **not** redefine R\*.

---

## Current position

R* vocabulary (canonical):

```text
PHASE_LIFECYCLE (*_STATUS) = NOT_STARTED | OPEN | COMPLETE
PHASE_RESULT (*_RESULT)    = PASS | FAIL | ACCEPT | REJECT | PARITY | WIN | LOSS | INCONCLUSIVE | …
```

```text
PROJECT_GLOBAL_PHASE =
  R5 COMPLETE (PASS) — private v0.4.3 published
  (R0–R4 COMPLETE; BASIC_PRODUCT_COMPLETENESS_GATE=PASS; GRC_P0 COMPLETE;
   PRE_R5_GRC_READINESS=PASS; R6 NOT_STARTED)

R0 = COMPLETE
R1 = COMPLETE
R2 = COMPLETE (PASS)
R3 = COMPLETE (PASS — PARITY)
R4 = COMPLETE (PASS)
R5 = COMPLETE (PASS)
R6 = NOT_STARTED

R0_STATUS = COMPLETE
R1_STATUS = COMPLETE
R2_STATUS = COMPLETE
R2_RESULT = PASS
R2_CLOSED = YES
R3_STATUS = COMPLETE
R3_RESULT = PASS
R3_CLOSED = YES
R3_COMPETITIVE_RESULT = PARITY
R4_STATUS = COMPLETE
R4_RESULT = PASS
R4_CLOSED = YES
R5_STATUS = COMPLETE
R5_RESULT = PASS
R5_CLOSED = YES
R6_STATUS = NOT_STARTED

PROJECT_PHASE_1_STATUS = NOT_CLOSED
PROJECT_PHASE_2_STATUS = NOT_STARTED
```

---

## R0 — Clean baseline

```text
R0_STATUS = COMPLETE
```

**Status:** `COMPLETE`

Delivered:

- Worktree authority on `/Volumes/Lexar/Cursor/exyonq-lab`
- Branch `release/v0.4.3-integration` @ product parent `fd41b761…` + docs `b3a24ca…`
- Product baseline = v0.4.2 + Changeset A + Changeset B
- Tracked tree clean of ambient product dirt (untracked `study/` preserved intentionally)

Exit criteria (met): known HEAD, known commits, no silent merge of dirty legacy tree.

---

## R1 — Documentation and status canonicalization

```text
R1_STATUS = COMPLETE
```

**Status:** `COMPLETE`

Delivered:

- [`PROJECT_STATUS.md`](PROJECT_STATUS.md) — operational truth  
- [`ROADMAP.md`](ROADMAP.md) — this file  
- [`DOCUMENTATION_INDEX.md`](DOCUMENTATION_INDEX.md) — inventory + taxonomy  
- Commit: `docs(project): establish canonical status and roadmap` (`b3a24ca…`)

---

## R2 — v0.4.3 qualification

```text
R2_STATUS = COMPLETE
R2_RESULT = PASS
R2_CLOSED = YES
```

**Status:** `COMPLETE` · **Result:** `PASS` (R2E closed 2026-08-02)

R2 baseline → R2A–R2C → R2D dual-arch → R2E Darwin KD3 harness close.
Linux/OCI product evidence remains @ `2f8e869…`. R2E added test-only
`proxy_metrics_assert_guard` (`2849d06…`) for process-wide PROXY_HTTP_501 races.
Evidence: `.exyonq-local/tmp/r2d-20260801/`, `.exyonq-local/tmp/r2e-20260801/`.

### Demonstrated

| Item | Result |
|------|--------|
| fmt / workspace check | PASS |
| workspace tests | PASS Darwin (R2E ws-3) + Netcup + Oracle |
| Clippy supported gates | PASS Darwin+Linux |
| cargo audit / deny | WAIVED OPTION_B `RUSTSEC-2026-0222` (not fixed; expiry v0.4.4) |
| SECINT / Wasm / P4 / plan12 / KD3 | PASS Darwin targeted + Linux dual-arch |
| static / proxy / FastCGI / TLS / H2 / H3 / reload | PASS Netcup + Oracle |
| release build `--locked` | PASS dual-arch |
| OCI amd64 / arm64 build+version | HISTORICAL R2D class A (NOT_GATE_CLOSING); runtime E2E required |

### R2 checklist

1–8. Closed (including Darwin workspace after R2E). Freeze → **R5** (not automatic).

```text
R2_STATUS = COMPLETE
R2_RESULT = PASS
R2_CLOSED = YES
PRODUCT_QUALIFIED_REVISION = 2f8e869f97c36e18ca7976d673dd6db2aa16e45e
R2_FINAL_QUALIFICATION_HEAD = 9176c374876d7c4bf503db1c1a02c885bb02fad9
R2_NEXT_ACTION = STOP — R2 complete; R3 already COMPLETE separately
Do not auto-open R4 / R5
```

---

## R3 — P4 valid proxy benchmark

```text
R3_STATUS = COMPLETE
R3_RESULT = PASS
R3_CLOSED = YES
R3_COMPETITIVE_RESULT = PARITY
R3_PRODUCT_BASELINE = f0b2d67
R3B/C/D = PASS
R3E_AMD64_CLASSIFICATION = PARITY
R3E_ARM64_CLASSIFICATION = PARITY
R3E_OVERALL_CLASSIFICATION = PARITY
FIRST_DUAL_ARCH_RUN = INVALID_HARNESS_EMFILE (NON_CANONICAL / HISTORICAL_ONLY)
CANONICAL_RERUN_COMPLETED = YES
NOFILE_FIX_APPLIED = YES
PERFORMANCE_COMPILER_STATUS = ACCEPT
R3_HTML = study/reports/r3-exyonq-vs-ols-fixed-rate-parity.html
R3_PUBLIC_CLAIM_ALLOWED = YES (fixed-rate PARITY only; PUBLICATION_STATUS=FORBIDDEN)
R4_OPEN = NO
R4_READY_TO_OPEN = YES
```

**Status:** `COMPLETE` · **Result:** `PASS` · **Competitive:** `PARITY` (2026-08-03)

Delivered:

- BASIC_PRODUCT_COMPLETENESS_GATE = PASS (prerequisite)  
- HTTP/3 POST body fix `8b702a2` dual-arch PASS  
- R3A mock infra reuse (class A)  
- R3B/C capacity + R3D topology neutrality PASS on Netcup+Oracle  
- R3E competitive ExyonQ vs OLS fixed-rate PARITY dual-arch @ `f0b2d67`  
- R3F reconciliation; performance-compiler ACCEPT  
- First dual-arch attempt preserved as INVALID_HARNESS_EMFILE (non-canonical)

Evidence (local): `.exyonq-local/tmp/r3-20260803/`; reconciliation `study/reports/R3F_RECONCILIATION.md`.

```text
R3_STATUS = COMPLETE
R3_RESULT = PASS
R3_CLOSED = YES
R3_COMPETITIVE_RESULT = PARITY
R3_READY_TO_CLOSE = YES
PRODUCT_CODE_CHANGED = NO
R3_NEXT_ACTION = STOP — do not auto-open R4 / R5 / GRC-P1
```

Exit criteria: **met** (sealed competitive packet + ACCEPT). Do **not** rerun R3E.

---

## R4 — Kubernetes baseline replay and OPEN-002 decision

```text
R4_STATUS = COMPLETE
R4_RESULT = PASS
R4_CLOSED = YES
```

**Status:** `COMPLETE` (`RESULT=PASS`)

Facts:

- P2A/P2B cherry-picked onto baseline `0fed18d`  
- `P2B_OPEN_002 = CLOSED` (`1d815fe` — eligible==1 → Single)  
- Productive acceptance on canonical R4 tip  
- `K8S_P2C_READY_TO_OPEN = YES` but `K8S_P2C_OPENED = NO`  

Exit criteria met: integrate + accept. P2C not auto-opened.

---

## R5 — v0.4.3 freeze and release

```text
R5_STATUS = COMPLETE
R5_RESULT = PASS
R5_CLOSED = YES
R5_OPENED = YES
PUBLIC_RELEASE = v0.4.3
FINAL_PUBLISHED_RELEASE_HEAD = 3a8af75dd0939e5aae2f4d09b842573499e15a1c
SUPERSEDED_RELEASE_CANDIDATE = f25137bff3b6fc0d4fc1b7f4c6d736b100fae741
GITHUB_RELEASE_STATUS = PUBLISHED
GHCR_V043_STATUS = PUBLISHED
GHCR_V043_DIGEST = sha256:ab02b5ffc3d54407a56b6edbb318d80a077da0f431970984408d82415217f2e5
LATEST_TAG_MUTATED = NO
SSH_TAG_SIGNATURE_STATUS = PASS
COSIGN_BLOB_SIGNATURE_STATUS = PASS
COSIGN_OCI_SIGNATURE_STATUS = PASS
RUSTSEC_2026_0222_EXCEPTION_STATUS = ACTIVE_WAIVER
VULNERABILITY_FIXED = NO
WAIVED_FOR_V043 = YES
EXPIRY_REVIEW = v0.4.4
```

**Status:** `COMPLETE` (PASS) — private `v0.4.3` published 2026-08-04.

Depends (met):

- R2 COMPLETE with RESULT=PASS  
- R3 COMPLETE with RESULT=PASS  
- R4 COMPLETE with RESULT=PASS  
- PRE_R5_GRC_READINESS=PASS  
- Explicit owner authorization for R5 qualification and final publication ceremony  

Published surfaces:

```text
TAG = v0.4.3 (SSH-signed; target 3a8af75…)
GITHUB_RELEASE = https://github.com/ExyonQ/ExyonQ/releases/tag/v0.4.3
GHCR = ghcr.io/exyonq/exyonq:0.4.3 @ sha256:ab02b5ff…
latest = unchanged
```

---

## R6 — Phase 2 admission

```text
R6_STATUS = NOT_STARTED
```

**Status:** `NOT_STARTED`

Phase 2 does **not** open merely because “Phase 1 feels done.”

### Verifiable gates that open Phase 2

All of the following must be true:

1. `PROJECT_PHASE_1_STATUS = CLOSED` recorded in `PROJECT_STATUS.md` with owner sign-off  
2. R5 either completed (tag exists under authorized release) **or** Phase 1 closed with an explicit “no tag yet / staging hold” decision that still closes Phase 1  
3. Release scope for the Phase 1 product line is frozen (no silent reopen of A/B integrity fixes)  
4. P4 either competitively closed (R3) **or** formally deferred with owner text  
5. K8S either integrated+accepted (R4) **or** formally deferred with owner text  
6. Feature Admission Gate answers recorded for the first Phase 2 initiative (problem, metric, complexity, rollback)  
7. No open Critical/High security findings without valid waiver  

Until then:

```text
PROJECT_PHASE_2_STATUS = NOT_STARTED
```

---

## Track ID taxonomy (non-global)

| Prefix | Meaning |
|--------|---------|
| `R*` | Global roadmap (this document) |
| `BV04-P*` | Benchmark v0.4.x scenarios |
| `K8S-P*` | Kubernetes wedge phases |
| `P14*` | Release / signing historical track |
| `K*` / `KD*` | Kernel / core historical development |
| `MR*` | Modern-rival validation |

---

## Change control

Updates to this file require:

- Consistency with `PROJECT_STATUS.md`  
- No silent status upgrades (e.g. R3 must not become COMPLETE without RESULT=PASS and sealed competitive packet)
- No new parallel roadmap files  
