# V044 Phase 6F — Dual-arch WordPress qualification

```text
WIP = V044_PHASE6F_DUAL_ARCH_WORDPRESS_QUALIFICATION
CASE = P6F-A
PARENT_TERMINAL = P6EROUTE-A (IMMUTABLE)
AMD64_EVIDENCE = REUSED_P6EROUTE_A (20260831T210044Z / 5d4b6bdf…)
ARM64_VALID_RUN = 20260831T213133Z
ARM64_BINARY_SHA256 = 7fae16d70330f1833c8a51fa0952050321798564764b252faeed83275aa16953
ARM64_EXECUTION_MODE = NATIVE
PRODUCT_MUTATION = NO
WORDPRESS_ARCHITECTURE_SCOPE = LINUX_AMD64_AND_LINUX_ARM64_VERIFIED
```

## Question answered

The same supported CFD WordPress production path (front-controller, DirectoryIndex,
pretty permalinks, `/wp-admin/`, auth, POST, REST, stable symlink containment,
KEEP_CONN, reload, max_conn fail-closed, recovery) works on **native Oracle ARM64**
with source-equivalent CFD digests to the P6EROUTE-A AMD64 qualification.

This is **not** a performance comparison. Binary SHA differs by architecture as required.

## Environment (ARM64)

| Component | Version |
| --- | --- |
| Host | oracle-quasar (`vnic-quasar`) aarch64 |
| Kernel | 6.17.0-1018-oracle |
| Rust | 1.97.1 / `aarch64-unknown-linux-gnu` |
| PHP-FPM | 8.3.6 |
| MariaDB | 10.11.14 |
| WordPress | 7.1 (official tarball) |

Invalid prior run `20260831T212851Z`: missing `php-mysqli` (environment); fixed and re-run.

## Residuals preserved

```text
SYMLINK_TOCTOU_STATUS = BOUNDED_RESIDUAL
FCGI_POST_PROBE_FIRST_WRITE_RESIDUAL = PRESERVED_IN_FUTURE_REVIEW_LEDGER
```

Historical P6E-C / P6EROUTE-B / P6EROUTE-A documents are not rewritten.

## Evidence

```text
EVIDENCE = .exyonq-local/evidence/phase6f-dual-arch-wordpress-qualification/20260831T213133Z/
TERMINAL = docs/governance/v044-phase6f-dual-arch-wordpress-qualification-terminal-block.txt
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
