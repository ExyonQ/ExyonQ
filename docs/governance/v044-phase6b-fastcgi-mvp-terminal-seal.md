# V044 Phase 6B FastCGI MVP Terminal Seal

```text
WIP                          = V044_PHASE6B_FASTCGI_MVP_TERMINAL_SEAL
PARENT                       = V044_PHASE6B_SYNC_CFD_NATIVE_FASTCGI_MVP
PARENT_CASE                  = P6B-B
CASE                         = P6BSEAL-B
DATE_UTC                     = 2026-08-30
PLATFORM                     = LINUX_AMD64_NETCUP
PRODUCT_MUTATION             = NO
PERFORMANCE_OPTIMIZATION     = NO
RIVAL_CAMPAIGN               = NO
PHASE_6C_AUTHORIZED          = NO
SEAL_CLASS                   = POSTHOC_DOCUMENTATION_RECONCILIATION
  + NARROW_REAL_RUNTIME_PROOF
PRE_MUTATION_ARCHITECTURE_REVIEW = FAIL
PRE_MUTATION_PROOF           = MISSING
FORMAL_ADR                   = 038
OWNER_TEMPLATE_NAME          = "ADR-021 Phase6B"   # NON_AUTHORITATIVE
ADR021_BINDING               = Safe H1 upstream reuse (governance ADR-021)
ENTRY_HEAD                   = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE                   = ac059750141ee48f141c543cccbd641652bf49fb
TERMINAL_HEAD                = d94856663f42ffce16a3c0ccc1291feb4d7083e4
TERMINAL_TREE                = UNCOMMITTED_WIP (Phase6B product sources still untracked vs HEAD)
TERMINAL_EVIDENCE_RUN        = 20260830T142958Z
PRIOR_CAUSAL_RUN             = 20260830T142927Z (N=100)
CANONICAL_MERGE              = NO
PUSH                         = NO
TAG                          = NO
RELEASE                      = NO
PUBLICATION                  = NO
```

## 1. Purpose

Close remaining epistemic gaps before any Phase 6C work:

1. Reconcile ADR-021 **governance analysis contract** naming with ADR-038 (no silent “N/A_USE_ADR038”).
2. Prove causal KEEP_CONN reuse against **real PHP-FPM** with an independent backend accept oracle.
3. Reconfirm generation drain + backend-failure fail-closed on real FPM.
4. Carry forward nonblocking P3 residuals with individual classification.

Non-goals: Phase 6C implementation, WordPress, Module API, rival FastCGI benchmarks, merge/push/tag.

## 2. Governance gap (honest)

```text
ADR021_GOVERNANCE_REQUIREMENT            = INCOMPLETE
PHASE6B_PRE_MUTATION_ARCHITECTURE_REVIEW = FAIL
ADR_COLLISION_REASON                     = ADR021_IDENTIFIER_ALREADY_ASSIGNED
FORMAL_ADR_FOR_PHASE6B                   = 038
GAP_CLASS                                = PRE_MUTATION_PROOF_MISSING
POSTHOC_DOCUMENTATION_REPAIR             = YES (this seal + ADR-038 POSTHOC section)
PRE_MUTATION_ACCEPT_REWRITTEN_AS_EXISTING = FORBIDDEN
```

Vertex architect field audit: ADR-038 has problem/alternatives/invariants/performance-nonclaim PRESENT; ownership, lifecycle SM, concurrency, Arc/Mutex budget, dual-boundary, rollback, SSOT, and most hot-path fields **MISSING** relative to ADR-021 discipline exemplars. Option D remains the recorded architecture decision (`STATUS=ACCEPTED`); that is **not** proof a complete pre-mutation Accept existed.

## 3. Claim-strength freeze

```text
FASTCGI_KEEP_CONN_SEMANTICS          = PASS   # BEGIN FCGI_KEEP_CONN + clean put path
FASTCGI_CLEAN_REUSE_COMPONENT        = PASS   # pool take/put/probe + cargo TrailingData
FASTCGI_REAL_PHP_FPM_CAUSAL_REUSE    = PASS   # Netcup SYN oracle 20260830T142958Z
P6B_MAY_CLAIM                        = MINIMAL_REAL_CFD_FASTCGI_MVP + CAUSAL_KEEP_CONN_NETCUP
P6B_MUST_NOT_CLAIM                   = WordPress | Module | FastCGI frontier | shipping complete
  | multi-tenant Unix peercred | pre-mutation architecture Accept
SECURITY_AUDIT_CEILING               = PASS_WITH_NONBLOCKING
LOGIC_AUDIT_CEILING                  = PASS_WITH_FINDINGS
ORACLE_ARM64_SHIPPING_ADMISSION      = NOT_IN_SCOPE
```

## 4. Causal KEEP_CONN (Netcup amd64)

```text
ORACLE                 = tcpdump SYN to 127.0.0.1:FPM_PORT (independent of ExyonQ pool counters)
CLIENT_H1_KEEPALIVE    = NO (Connection: close each request)
FASTCGI_BACKEND_KEEP_CONN = YES (measured)
RUN_ID                 = 20260830T142958Z
KEEP_CONN_REQUESTS     = 1000
PHP_FPM_ACCEPTED_CONNECTIONS (SYN) = 1
FASTCGI_BACKEND_ACCEPTS_PER_REQUEST = 0.001
FASTCGI_REAL_PHP_FPM_REUSE_RATIO    = 1000
CROSS_REQUEST_FASTCGI_CONTAMINATION = 0
REAL_DYNAMIC_PHP       = PASS (monotonic n + unique nonces)
REAL_GENERATION_DRAIN  = PASS (gen 1 → 2)
REAL_BACKEND_FAILURE   = PASS (502 after FPM kill)
BINARY_SHA256          = 8e7d5f1a454d78636dd3db8b84433418ac8d024f28293696ba8218d8ed0ff8b4
EVIDENCE               = .exyonq-local/evidence/phase6b-fastcgi-mvp-terminal-seal/20260830T142958Z/
SCRIPT                 = scripts/reality/run-phase6b-terminal-seal-keepconn-netcup.sh
```

Prior N=100 run `20260830T142927Z` also PASS (SYN=1).

## 5. Dirty reuse honesty

```text
DIRTY_REUSE_COMPONENT_TESTS = PASS
  (exyonq-fastcgi-wire TrailingData; phase6b_fcgi trailing_data_forces_discard_error)
DIRTY_CONNECTION_REUSE_COUNT (real PHP-FPM adversarial delayed trailing) = NOT_MEASURED_IN_PHP_FPM_SEAL
FAILED_BACKEND_CONNECTION_REUSE = 0 (observed 502/504 after FPM kill; no success via stale pool)
SAFE_CHECKIN / SAFE_CHECKOUT = PASS (put+take probe; FCGI-RACE-01 residual acknowledged)
```

Do not treat hardcoded zeros as measured adversarial dirty isolation under delayed trailing.

## 6. SG-FCGI-06

```text
SG_FCGI_06_SOCKET_TRUST     = QUALIFIED_NONBLOCKING
UNIX_SOCKET_TRUST_MODEL     = SINGLE_TENANT_ADMIN_FS
  absolute path at control + connect; client cannot choose socket; no peercred claim
```

Seal Netcup harness used TCP `127.0.0.1` FPM — peercred N/A for that transport.

## 7. P3 ledger (four items)

| ID | Description | Classification | Blocks P6C | Blocks shipping |
| --- | --- | --- | --- | --- |
| P6B-P3-01 | KEEP_CONN causal reuse unmeasured | **CLOSED_BY_CURRENT_EVIDENCE** (Netcup SYN) | NO | NO (for causal claim) |
| P6B-P3-02 | SG-FCGI-06 no peercred | OPEN_NONBLOCKING_PHASE6 / QUALIFIED | NO | YES if multi-tenant Unix |
| P6B-P3-03 | FCGI-RACE-01 checkout→BEGIN | OPEN_NONBLOCKING_PHASE6 → DEFERRED_TO_P6C | NO | NO for MVP |
| P6B-P3-04 | `script_suffix` unused; `reuse_ok` always true | OPEN_NONBLOCKING_PHASE6 | NO | YES for suffix/CMS claims |

```text
SUPPORTED_CONTRACT_MUST_FIX_COUNT = 0
OPEN_P0 = 0
OPEN_P1 = 0
OPEN_P2 = 1  # harness SUMMARY honesty (fixed labeling; non-product)
OPEN_P3 = 3  # after closing P6B-P3-01
```

## 8. Gates (this seal)

```text
CARGO_FMT_CHECK            = PASS
CARGO_CHECK_WORKSPACE      = PASS
CARGO_CHECK_FEATURE_MATRIX = PASS (cfd-dataplane/wire/control/gen + http3-provider-quiche)
CARGO_CLIPPY_CANONICAL     = PASS (workspace -D warnings + exyonq-wasm-host)
TARGETED_FASTCGI_TESTS     = PASS (wire 11; phase6b_fcgi 5)
CARGO_TEST_WORKSPACE       = PASS (1483 passed; 0 failed; Darwin lab tree)
ZERO_FAKE                  = PASS
NO_SMOKE                   = PASS
ANTI_KOBAYASHI             = PASS
SECURITY_AUDIT             = PASS_WITH_NONBLOCKING
INDEPENDENT_LOGIC_AUDIT    = PASS_WITH_FINDINGS
```

## 9. Phase 6C admission (not authorized)

```text
PHASE_6C_FEASIBLE   = CONDITIONAL
PHASE_6C_AUTHORIZED = NO
PROPOSED_PHASE6C_SCOPE =
  Prefer: FCGI-RACE-01 hold-READABLE / pre-BEGIN re-probe (pool hardening)
  + optional streaming / larger body pressure if product needs it
  Do NOT invent redundant body/pool work already closed for MVP GET ≤64KiB
  Do NOT open FastCGI rival frontier or WordPress under P6C without new auth
```

## 10. Notion reconciliation facts

```text
NOTION_RECONCILIATION_REQUIRED = YES
FACTS:
1. Seal CASE = P6BSEAL-B (not A): ADR pre-mutation incomplete + nonblocking P3 residuals.
2. Formal ADR = 038; "ADR-021 Phase6B" is non-authoritative template name.
3. ADR-021 remains H1 upstream reuse; ADR021_GOVERNANCE_REQUIREMENT = INCOMPLETE for Phase6B analysis completeness.
4. Causal KEEP_CONN PASS: 1000 req / 1 backend SYN on Netcup; independent tcpdump oracle.
5. SG-FCGI-06 remains QUALIFIED_NONBLOCKING.
6. PHASE_6C_AUTHORIZED = NO; MERGE/PUSH/TAG/RELEASE = NO.
7. Seal doc = docs/governance/v044-phase6b-fastcgi-mvp-terminal-seal.md
8. Evidence RUN = 20260830T142958Z under .exyonq-local/evidence/phase6b-fastcgi-mvp-terminal-seal/
```

## 11. Stop

```text
STOP_AFTER_TERMINAL = YES
DOES_P6B_TERMINAL_SEAL_INCREASE_MIGRATION_VALUE = YES
  # Removes false belief that KEEP_CONN was only semantic/component; proves real FPM reuse causality.
FASTCGI_PERFORMANCE_FRONTIER = NOT_MEASURED
```
