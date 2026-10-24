# V044 Phase 6 — FastCGI pre-WordPress blocker repair and attribution close

```text
WIP = V044_PHASE6_FASTCGI_PRE_WORDPRESS_BLOCKER_REPAIR_AND_ATTRIBUTION_CLOSE
CASE = P6PREWP-B
PARENT = P6DRC-B (IMMUTABLE)
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
ADR = docs/adr/041-phase6-pre-wordpress-fastcgi-residual-honesty.md
WORDPRESS = NO
```

## Two tracks (not conflated)

| Track | Problem | Outcome |
|-------|---------|---------|
| **A** | FastCGI residuals A1/A2/A3 | A1 qualified no-fix; A2 fail-closed; A3 dead field removed |
| **B** | Workspace attribution + proxy_sdp hang | B1 attributable worktree; hang ENVIRONMENTAL; WS test PASS |

## Track A ledger

| ID | Classification | Mutation | Status |
|----|----------------|----------|--------|
| A1 SG-FCGI-06 | VALID_QUALIFICATION_NO_FIX_REQUIRED | NO | A1-B SINGLE_TENANT_ADMIN_FS sufficient; peercred future |
| A2 script_suffix | REAL_SUPPORTED_HARDENING (config honesty) | YES | Reject nonempty at control + IR; `CONFIG_ACCEPTED_BUT_SEMANTICALLY_IGNORED=NO` after fix |
| A3 FCGI reuse_ok | DEAD_OR_UNUSED_CODE_CLEANUP | YES | Removed; `FcgiPoolMap::put` remains checkin SSOT |

## Track B ledger

```text
ATTRIBUTABLE_SOURCE_METHOD = B1_DETACHED_WORKTREE_PLUS_BOUNDED_OVERLAY
BASE_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
BASE_TREE = ac059750141ee48f141c543cccbd641652bf49fb
RESULTING_TREE_ID = a46545a8fa6715e751dc04c1cec95b21386c0b55
PATCH_SHA256_FINAL = fa17449efef8a96127243a126d6779e7421572a56e99d94039b03d0cba075ac2
SOURCE_PROVENANCE = PASS_BOUNDED
SOURCE_RECONSTRUCTION = PASS
PROXY_SDP_HANG_CLASSIFICATION = ENVIRONMENTAL_DEPENDENCY
PROXY_SDP_HANG_REPAIR = NONE
CARGO_TEST_WORKSPACE = PASS (1502) on attributable worktree
CARGO_CLIPPY_CANONICAL = FAIL_PRE_EXISTING_BASE_HEAD_PROXY_SDP
CARGO_FMT_CHECK = FAIL_PRE_EXISTING_BASE_HEAD_RUSTFMT
```

## Netcup regression (bounded)

```text
BINARY_SHA256 = a6107ce62696ac45cdcb692a8df9404ab2752bcca677c2b26ec946347af3dcb0
KEEP_CONN = PASS (200 requests / 1 backend SYN)
SUFFIX_REJECT = PASS
TTL/MAX/GEN policy oracles = NOT_MEASURED (survival-after-publish only; LA-001)
```

## Audits

| Audit | Verdict |
|-------|---------|
| Architecture | PASS |
| Security | PASS_WITH_NONBLOCKING |
| GRC | PASS |
| Logic | PASS_WITH_FINDINGS (P2 evidence honesty remediated in harness labels) |

## WordPress

```text
FASTCGI_PRE_WORDPRESS_BLOCKERS = 0
WORDPRESS_CURRENT_HEAD_QUALIFICATION_FEASIBLE = CONDITIONAL
WORDPRESS_AUTHORIZED = NO
```

## Evidence

`.exyonq-local/evidence/phase6-fastcgi-pre-wordpress-blocker-repair-attribution-close/20260830T225734Z/`
