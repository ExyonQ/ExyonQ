# V044 Phase 6D — Residual requalification close

```text
WIP = V044_PHASE6D_RESIDUAL_REQUALIFICATION_CLOSE
CASE = P6DRC-B
PARENT_STRESS = P6D-C (IMMUTABLE — do not rewrite)
PARENT_REPAIR = P6DR-B (IMMUTABLE — do not rewrite)
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
PRODUCT_MUTATION = NO
```

## Purpose

Epistemic close of residual evidence/attribution gaps after:

- **P6D-C** — stress qualification stopped with `PRODUCT_REPAIR_REQUIRED`
- **P6DR-B** — `P6D-L1-001` repaired (`D_REFRESH_POLICY_AND_SWEEP_INVALID_IDLE`) with nonblocking qualifications

This WIP is **not** a full P6D campaign replay, not FastCGI optimization, not WordPress, not Module API, not rivals.

## Historical terminals (preserved)

| Case | Status |
|------|--------|
| P6D-C | `COMPLETE_WITH_PRODUCT_REPAIR_REQUIRED` — PRESERVED |
| P6DR-B | `P6D_L1_001_REPAIR_CLOSED_WITH_NONBLOCKING_QUALIFICATIONS` — PRESERVED |

## Repaired defect (not re-proven)

```text
FINDING_ID = P6D-L1-001
FINDING_STATUS = CLOSED
ROOT_CAUSE = or_insert_with_stale_pool_runtime_policy
CHOSEN_REPAIR = D_REFRESH_POLICY_AND_SWEEP_INVALID_IDLE (ADR-040)
```

## Attributable workspace method

```text
ATTRIBUTABLE_WORKSPACE_METHOD = A_CANONICAL_AMBIENT_DIRTY_PARTIAL
```

Canonical lab worktree retains ambient dirt (~150 porcelain paths + untracked CFD).
Isolated clean Method B was not required for **qualified** close; Method B would still be
hard under ambient ownership and would not include untracked CFD without copy.

| Gate | Lab (ambient dirty) | Netcup `/root/exyonq-p6d-fcgi` | Classification |
|------|---------------------|-------------------------------|----------------|
| CFD package fmt/check/clippy/test | PASS | N/A (binary identity) | Repair-surface attributable |
| `cargo fmt --check` workspace | PASS | — | Ambient Method A_PARTIAL |
| `cargo check --workspace --all-targets` | PASS | FAIL exit 101 (`sendfile_fsm` private) | Netcup FAIL = PRE_EXISTING; REPAIR_CAUSED=NO |
| Canonical feature matrix (default + `http3-provider-quiche`) | PASS | — | `--all-features` = INVALID probe exit 101 |
| Canonical clippy workspace | PASS | — | Ambient Method A_PARTIAL |
| `cargo test --workspace` | ABORTED hang at `exyonq_proxy_sdp` | — | PRE_EXISTING ambient; REPAIR_CAUSED=NO |

Evidence: `.exyonq-local/evidence/phase6d-residual-requalification-close/20260830T213810Z/`

## Source / binary identity

```text
REQUALIFICATION_SOURCE_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
REQUALIFICATION_SOURCE_TREE = ac059750141ee48f141c543cccbd641652bf49fb
REPAIR_PATCH_IDENTITY = Design D ensure_pool policy refresh (lab↔netcup rustfmt-only)
BINARY_SHA256 = 3eb7a478233da06e836b2e3313ebc3151ce77bb4253e6fd9fcb6de4d076947bc
BINARY_IDENTITY = PASS (Netcup observed == P6DR-B repair)
SOURCE_PROVENANCE = PASS_BOUNDED
  # Design D present on lab disk + Netcup binary identity.
  # CFD crate is ambient/untracked overlay — not a member of ENTRY_TREE ac059750.
  # Lab↔Netcup fcgi_pool byte hash differs by rustfmt only.
```


## Materiality of residual `NOT_MEASURED` runtime fields

| Field | REQUIRED_FOR_P6D_CLOSE | Classification | Value |
|-------|------------------------|----------------|-------|
| DIRTY_CONNECTION_REUSE | NO | ALREADY_PROVEN_P6C + NONBLOCKING PHP-FPM delayed-trailing | NOT_MEASURED |
| DUPLICATE_PHP_EXECUTION | NO | NONBLOCKING_CARRY_FORWARD | NOT_MEASURED |
| GENERATION_INVALID_REUSE | NO | ALREADY_PROVEN_P6C + NONBLOCKING stress counter | NOT_MEASURED |
| RETRY_AFTER_COMMIT | NO | ALREADY_PROVEN_P6C CommitStage | NOT_MEASURED |

Do **not** promote `NOT_MEASURED` → `PASS`/`0`.

## Reused runtime evidence (repair VALID only)

```text
REUSE_ROOT = .exyonq-local/evidence/p6d-l1-001-fastcgi-pool-reload-refresh-repair/20260830T204920Z/
BINARY = 3eb7a478…
```

| Claim | REUSE_VALIDITY |
|-------|----------------|
| S8 TTL eviction + KEEP_CONN 500/1 | VALID |
| S9 churn/contam/FD (BOUNDED occupancy) | VALID (BOUNDED) |
| FD / RSS / THREAD stability | VALID |
| Historical stress `194007Z` / binary `926c6289…` | INVALID for post-repair CURRENT |
| POOL_MAP_ENTRY_STABILITY as measured counter | NOT_MEASURED (no harness oracle) |
| READINESS_TOKEN_STABILITY as measured series | NOT_MEASURED (no harness oracle) |

## Fresh gates (this WIP)

- CFD package gates PASS (`20260830T213810Z`)
- P6B/P6C/P6DR package regression (`exyonq-cfd-dataplane` + `exyonq-fastcgi-wire`) PASS
- Harness SUMMARY honesty re-audit: unsupported fields remain `NOT_MEASURED`; `UNSUPPORTED_DEFAULT_ZERO_PASS_COUNT = 0`

## P6D current classification after this terminal

```text
P6D_CURRENT_STATUS = CLOSED_REQUALIFIED_WITH_NONBLOCKING_QUALIFICATIONS
CASE = P6DRC-B
```

### Nonblocking qualifications (explicit)

1. Ambient Method A_PARTIAL — lab workspace green ≠ clean repaired-only tree; forbids P6DRC-A
2. Netcup workspace check FAIL = PRE_EXISTING (`sendfile_fsm`); REPAIR_CAUSED=NO
3. Workspace `cargo test` ABORTED hang on ambient `exyonq_proxy_sdp`; REPAIR_CAUSED=NO; CFD package tests PASS
4. Four residual runtime counters remain NOT_MEASURED with REQUIRED_FOR_P6D_CLOSE=NO
5. S9 max occupancy system oracle BOUNDED (contam/FD), not full occupancy shrink instrumentation
6. S8 SYN-on-lo secondary oracle insufficiency (SPORT/ActiveOpens primary) — carry from P6DR
7. `FCGI_POST_PROBE_TO_FIRST_WRITE_MICRO_WINDOW` preserved future review
8. `SG_FCGI_06_PEERCRED` OPEN_NONBLOCKING_QUALIFIED
9. `SCRIPT_SUFFIX_UNUSED_PLUS_REUSE_OK_ALWAYS_TRUE` OPEN_NONBLOCKING
10. Claim ceiling BOUNDED Netcup amd64 — not `VERIFIED_REAL_PRODUCTION`

## Audits

| Audit | Verdict |
|-------|---------|
| Architecture (Vertex) | PASS |
| Security | PASS_WITH_NONBLOCKING |
| GRC | PASS (after seal artifacts) |
| Independent logic | see terminal block |

## WordPress / next phase

```text
WORDPRESS_FUTURE_QUALIFICATION = CONDITIONAL
NEXT_PHASE_AUTHORIZED = NO
PHASE_6E_AUTHORIZED = NO
PHASE_7_AUTHORIZED = NO
```

Clean P6D residual close is prerequisite evidence, not permission.

## Docs / evidence

- This file + `v044-phase6d-residual-requalification-close-terminal-block.txt`
- Evidence: `.exyonq-local/evidence/phase6d-residual-requalification-close/20260830T213810Z/`
- Parents: P6D-C / P6DR-B terminals unchanged
- ADR-040 unchanged
