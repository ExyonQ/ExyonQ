# V044 Phase 6E — WordPress routing symlink same-binary reseal

```text
WIP = V044_PHASE6E_WORDPRESS_ROUTING_SYMLINK_SAME_BINARY_RESEAL
CASE = P6EROUTE-A
PARENT_TERMINAL = P6EROUTE-B (IMMUTABLE)
ANCESTOR = P6E-C (IMMUTABLE)
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
VALID_RUN = 20260831T210044Z
BINARY_SHA256 = 5d4b6bdf6c6439f0b98656e1e84c83ea6a5cd0640490bf310e9e8af7197b367b
PRODUCT_MUTATION = NO
NEW_ADR_REQUIRED = NO
SAME_BINARY_REAL_NETCUP_PROOF = SATISFIED
PHASE6E_WORDPRESS_CURRENT_STATUS = CLOSED_VERIFIED_REAL_PRODUCTION_PATH
```

## Gap closed

P6EROUTE-B closed DEFECT_1/2 on Netcup binary `039532…`, which **predated** the
explicit-`.php` `path_is_under_root` containment fix. This reseal built the
post-fix ambient CFD source once on Netcup, bound
`LOCAL_BINARY_SHA256 == REMOTE_BINARY_SHA256`, and ran:

1. Real filesystem symlink escape matrix (PHP / static / DI / FC / parent / chain)
2. In-docroot symlink control
3. Generic FC / DI / static / explicit PHP
4. Bounded WordPress pretty / `/wp-admin/` / status / KEEP_CONN

on **that exact binary**.

## Results

| Gate | Result |
|------|--------|
| BINARY_IDENTITY | PASS |
| DOCROOT_ESCAPE (stable snapshot, DI+FC) | 0 |
| SYMLINK_*_ESCAPE | BLOCKED |
| OUTSIDE_PHP_NON_EXECUTION | PASS |
| GENERIC_FRONT_CONTROLLER / DIRECTORY_INDEX | PASS |
| PRETTY_PERMALINK_POST / PAGE | PASS |
| WP_ADMIN_SLASH | PASS |
| WORDPRESS_FRONT_CONTROLLER | PASS |
| WORDPRESS_KEEP_CONN | PASS |
| CROSS_REQUEST_CONTAMINATION | MEASURED_0 (title/h1) |

```text
SYMLINK_TOCTOU_STATUS = BOUNDED_RESIDUAL
```

Concurrent symlink-swap between resolve and FPM open is **not** claimed
`PROVEN_SAFE` (PRODUCT_MUTATION=NO). No-policy legacy path remains ADR-043
prior behavior (out of this claim).

## Historical terminals

Do **not** rewrite P6E-C or P6EROUTE-B. They remain historically true.

## Evidence

```text
EVIDENCE = .exyonq-local/evidence/phase6e-wordpress-routing-symlink-same-binary-reseal/20260831T210044Z/
TERMINAL = docs/governance/v044-phase6e-wordpress-routing-symlink-same-binary-reseal-terminal-block.txt
```

```text
NEXT_WIP_AUTHORIZED = NO
PHASE_7_AUTHORIZED = NO
CANONICAL_MERGE = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION = NO
```
