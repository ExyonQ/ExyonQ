# V044 Phase 6G — Additional generic PHP application qualification

```text
WIP = V044_PHASE6G_ADDITIONAL_GENERIC_PHP_APP_QUALIFICATION
CASE = P6G-A
PARENT_TERMINAL = P6F-A (IMMUTABLE)
VALID_RUN = 20260831T221620Z
BINARY_SHA256 = 5d4b6bdf6c6439f0b98656e1e84c83ea6a5cd0640490bf310e9e8af7197b367b
PLATFORM = LINUX_AMD64_NETCUP
SECOND_PLATFORM = NOT_REQUIRED_FOR_INITIAL_CLOSE (owner)
PRODUCT_MUTATION = NO
HARNESS_MUTATION = YES_EVIDENCE_ONLY
TARGET_APP = Laravel
TARGET_APP_VERSION = skeleton laravel/laravel:13.10.1 / framework v13.29.0
GENERIC_PHP_APPLICATION_SCOPE = WORDPRESS_PLUS_SECOND_REAL_FRAMEWORK_VERIFIED
```

## Question answered

Can a second real PHP framework run correctly through the **current** CFD FastCGI /
routing path **without** application-specific ExyonQ product logic?

**Yes** on Linux AMD64 Netcup for Laravel 13 (front controller `public/index.php`,
static/PHP coexistence, dynamic routes, params, query, POST, CSRF, cookies/sessions,
MariaDB HTTP→DB, redirects, 404/405/500, backend/DB recovery, generation reload,
max_conn fail-closed, KEEP_CONN, shards 1/4, symlink stable containment).

This is independent of WordPress shape. CFD source digests match the P6F/P6EROUTE
AMD64 qualification binary (`5d4b6bdf…`). No Laravel/WordPress strings in CFD product.

### Honesty — “static asset” labeling (Phase 7 reconciliation)

```text
STATIC_VIA_CFD_PROXY_SIDECAR = YES
STATIC_CONTRACT = PROXY_FILESERVER_NOT_BACKENDKIND_STATIC
  # /assets/ + /favicon.ico were Cap033 longer Proxy prefixes → Python SimpleHTTPRequestHandler
  # NOT Cap067 / NOT BackendKind::Static (absent from CFD IR)
SEALED_TERMINAL_P6G_A = IMMUTABLE
  # Do not rewrite PASS keys on sealed run; this discloses causal path only (GRC-P6G-001)
```

## Environment

| Component | Value |
|-----------|-------|
| Host | netcup-bench (x86_64) |
| Kernel | 6.8.0-134-generic |
| Rust | 1.97.1 |
| PHP-FPM | 8.3.6 |
| MariaDB | 10.11.14 |
| Laravel | skeleton 13.10.1 / framework v13.29.0 |
| Composer lock SHA256 | `f4aaff77f6f0161ba5cf344932fea787aebf0f27bd9a92724f9091b795825789` |

## Residuals preserved

```text
SYMLINK_TOCTOU_STATUS = BOUNDED_RESIDUAL
P6F-A / P6EROUTE-A / WORDPRESS dual-arch = PRESERVED (not reopened)
TTL_RELOAD / GENERATION_RELOAD STRONG_CAUSAL = PRESERVED (bounded corroboration this run)
FCGI_MAX_CONN = 1_ONLY
```

## Invalid runs (disclosed)

| RUN_ID | Disclosure |
|--------|------------|
| 20260831T215544Z | INVALID_HARNESS_ROUTE_USE_DUPLICATE |
| 20260831T215703Z | INVALID_ENV_DB_NOT_APPLIED |
| 20260831T215743Z | INVALID_FPM_NO_INPUT_FILE_TMP_PERMS |
| 20260831T220235Z | INVALID_VAR_WWW_MODE_700 |
| 20260831T220315Z | INVALID_PORT_COLLISION_STALE_DATAPLANE |
| 20260831T220606Z | INVALID_HUNG_CONCURRENCY_NO_CURL_TIMEOUT_SUPERSEDED |
| 20260831T221225Z | INVALID_BARE_WAIT_HANG_SUPERSEDED |

## Migration value

```text
DOES_SECOND_PHP_APP_INCREASE_MIGRATION_VALUE = YES
```

Framework diversity (CMS vs Laravel front-controller) reduces WordPress-specific bias
and raises confidence that ADR-043 generic FC/DI routing is not app-shaped.

## Recommended next (not authorized)

```text
RECOMMENDED_NEXT_WIP = PHASE_7_STATIC_PROXY_COEXISTENCE
NEXT_WIP_AUTHORIZED = NO
PHASE_7_AUTHORIZED = NO
```

## Evidence

```text
EVIDENCE = .exyonq-local/evidence/phase6g-additional-generic-php-app-qualification/20260831T221620Z/
DOCS = docs/governance/v044-phase6g-additional-generic-php-app-qualification.md
TERMINAL = docs/governance/v044-phase6g-additional-generic-php-app-qualification-terminal-block.txt
HARNESS = scripts/reality/run-phase6g-laravel-qualification-netcup.sh
REMOTE = scripts/remote/v044-phase6g-additional-generic-php-app-qualification-netcup.sh
```

```text
CANONICAL_MERGE = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION = NO
NOTION_RECONCILIATION_REQUIRED = YES
```
