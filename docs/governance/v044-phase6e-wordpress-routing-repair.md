# V044 Phase 6E — WordPress routing repair

```text
WIP = V044_PHASE6E_WORDPRESS_ROUTING_REPAIR
CASE = P6EROUTE-B
  # Historical terminal — immutable. Current Phase 6E status via reseal:
  # docs/governance/v044-phase6e-wordpress-routing-symlink-same-binary-reseal.md
  # CASE P6EROUTE-A / CLOSED_VERIFIED_REAL_PRODUCTION_PATH
PARENT_TERMINAL = P6E-C (IMMUTABLE)
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
VALID_RUN = 20260831T172804Z
NETCUP_BINARY_SHA256 = 039532fde176e4bdc3a7a383fa5f6221d8e6373e39382abdf1cbdbdc35e02f41
SOURCE_CLASS = WORKTREE_AMBIENT_CFD_UNTRACKED
PRODUCT_MUTATION = YES_BOUNDED
ADR = 043
WORDPRESS_SPECIFIC_PRODUCT_FAST_PATH = NO
PHASE6E_WORDPRESS_CURRENT_STATUS_AT_THIS_TERMINAL =
  CLOSED_VERIFIED_REAL_PRODUCTION_PATH_WITH_NONBLOCKING_QUALIFICATIONS
PHASE6E_WORDPRESS_CURRENT_STATUS =
  CLOSED_VERIFIED_REAL_PRODUCTION_PATH
  # After P6EROUTE-A reseal (same-binary Netcup 20260831T210044Z); this file
  # remains the P6EROUTE-B historical record and is not rewritten as A.
```


## Repair

ADR-043 projects Plan-11 / htaccess-overlay **DirectoryIndex** + **front-controller**
semantics into CFD generation (`CFDRT004`) and resolves the executable script
**before** `execute_get`.

```text
DEFECT_1 = CFD_FRONT_CONTROLLER_PRETTY_PERMALINK_ROUTING → CLOSED
DEFECT_2 = CFD_DIRECTORY_INDEX_WP_ADMIN_SLASH → CLOSED
SUPPORTED_CONTRACT_MUST_FIX_COUNT = 0
```

Control-plane:

```text
fcgi-dir-index|pool_id|index.php[|…]
fcgi-front-controller|pool_id|/index.php
```

## Proof

| Gate | Result |
|------|--------|
| GENERIC_FRONT_CONTROLLER / DIRECTORY_INDEX | PASS (real PHP fixture) |
| PRETTY_PERMALINK_POST / PAGE | PASS |
| WORDPRESS_FRONT_CONTROLLER | PASS |
| WP_ADMIN_SLASH | PASS |
| WORDPRESS_PRETTY_404 | PASS |
| WORDPRESS_LARGER_DYNAMIC_RESPONSE (pretty) | PASS |
| WP_LOGIN / AUTH / AUTHENTICATED `/wp-admin/` | PASS |
| KEEP_CONN (ss ESTAB oracle) | PASS |
| CROSS_REQUEST_CONTAMINATION | MEASURED_0 (title/h1; nav false-positive corrected) |
| max_conn=6 fail-closed | PASS |
| ADR-042 / P6MAXHONEST-A | PRESERVED |
| P6EROUTE-LOGIC-001 (symlink escape) | CLOSED (unit); Netcup binary predates fix |

## Nonblocking qualifications (CASE B)

1. Netcup `BINARY_SHA256=039532…` closed WordPress FC/DI **before** explicit-`.php`
   symlink containment; containment is unit/code-proven on the post-fix ambient tree.
2. Darwin `foundation_matrix` ReadyTimeout = ENVIRONMENT (not repair-caused).
3. Media upload still `NOT_IN_CURRENT_SUPPORTED_SCOPE` (64 KiB).
4. Full `cargo --workspace` / feature-matrix not re-bound this terminal (CFD package
   fmt/check/clippy/unit PASS).

P6E-C remains historically true.

## Evidence

```text
EVIDENCE = .exyonq-local/evidence/phase6e-wordpress-routing-repair/20260831T172804Z/
ADR = docs/adr/043-phase6e-generic-front-controller-directory-index-routing.md
TERMINAL = docs/governance/v044-phase6e-wordpress-routing-repair-terminal-block.txt
```

```text
NEXT_WIP_AUTHORIZED = NO
PHASE_7_AUTHORIZED = NO
FASTCGI_PERFORMANCE_FRONTIER = NOT_MEASURED
CANONICAL_MERGE = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION = NO
```
