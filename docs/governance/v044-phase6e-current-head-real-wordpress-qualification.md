# V044 Phase 6E — CURRENT HEAD real WordPress qualification

```text
WIP = V044_PHASE6E_CURRENT_HEAD_REAL_WORDPRESS_QUALIFICATION
CASE = P6E-C
PARENT_TERMINAL = P6MAXHONEST-A (IMMUTABLE)
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
VALID_RUN = 20260831T142351Z
NETCUP_BINARY_SHA256 = 8da3d613e6597032b363dc9ddce233fbabdd73c6ecd9c48be79be4825f57af55
SOURCE_CLASS = WORKTREE_AMBIENT_CFD_UNTRACKED
  # CFD crates on Netcup sync disk; not in committed ENTRY_TREE
PRODUCT_MUTATION = NO
HARNESS_MUTATION = YES
DOC_MUTATION = YES
WORDPRESS = YES_REAL
PERFORMANCE_OPTIMIZATION = NO
ASYNC_FASTCGI = NO
PHASE_7_AUTHORIZED = NO
PUBLICATION = NO
```

## Purpose

Qualify whether CURRENT HEAD ExyonQ CFD FastCGI can serve a real WordPress
deployment (real PHP-FPM + real MariaDB) on Linux amd64 Netcup under the
current truthful contract (`max_conn=1`, serial `execute_get`, no async FastCGI).

## Environment

| Item | Value |
| --- | --- |
| Platform | LINUX_AMD64_NETCUP |
| OS/kernel | Ubuntu, 6.8.0-134-generic |
| CPU/RAM | 8 / ~15 GiB |
| PHP / FPM | 8.3.6 |
| Database | MariaDB 10.11.14 (local Netcup) |
| WordPress | 7.1 (official `latest.tar.gz`) |
| WP SHA256 | `05a5f89138f632b7329f1202f2a0553c5f7fe4daf8e4b9ca7ebae9b9466b9e86` |
| Theme | twentytwentyfive (core) |
| Plugins | none (`WORDPRESS_CORE_ONLY=YES`) |
| TLS | HTTP_ONLY |
| WAF | off for this run (optional CFD default) |

## Configuration under test

```text
FCGI_MAX_CONN = 1
FCGI_IDLE_TTL = 60000 (G2 idle changed to 45000 for bounded TTL corroboration)
FCGI_POOL_ID = 1
CFD_SHARDS = 1 then 4
FRONT_CONTROLLER = NOT_IMPLEMENTED_IN_CFD
STATIC = Proxy → local Python SimpleHTTPRequestHandler on docroot (no native Static BackendKind)
```

## Results (VALID_RUN)

| Gate | Result |
| --- | --- |
| REAL_WORDPRESS / PHP-FPM / DATABASE | PASS |
| WORDPRESS_INSTALL / FRONT_PAGE | PASS |
| STATIC_PHP_COEXISTENCE | PASS (via CFD Proxy sidecar; not native static) |
| PRETTY_PERMALINK_POST/PAGE | FAIL — PHP-FPM `File not found` |
| WORDPRESS_FRONT_CONTROLLER | FAIL — no try_files/rewrite/PATH_INFO |
| WP_LOGIN / AUTH_COOKIE / POST_BODY | PASS |
| WP_ADMIN (`/wp-admin/`) | FAIL — PHP-FPM `Access denied` (directory SCRIPT_FILENAME) |
| WP_REST_API | PASS via `index.php?rest_route=` |
| WORDPRESS_MEDIA_UPLOAD | NOT_IN_CURRENT_SUPPORTED_SCOPE (FCGI body 64 KiB) |
| KEEP_CONN | PARTIAL (50/200 OK with mixed broken routes; SYN pcap empty) |
| CROSS_REQUEST_CONTAMINATION | MEASURED_0_VACUITY_RISK_UNDER_PRETTY |
| WORDPRESS_CONCURRENCY | FAIL — all stages OK=0 at C=1..32 (pretty/query paths broken) |
| BACKEND hard fail + recover without ExyonQ restart | PASS |
| DB_FAILURE_SURVIVAL | PASS |
| GENERATION reload under WP | PASS |
| INVALID max_conn=6 publish fail-closed | PASS (ADR-042) |
| WORDPRESS_SHARD_1 / SHARD_4 | PASS_BOOT_GEOMETRY_ONLY (not concurrency proof) |

### Evidence corrections (harness false FAIL)

CGI probe JSON in VALID_RUN shows correct `SCRIPT_FILENAME`, `QUERY_STRING`, `REQUEST_URI`.
Harness `RESULTS.env` marked CGI FAIL due to grep of unescaped `DOCROOT` against JSON-escaped paths.
Corrected: SCRIPT_FILENAME / CGI / QUERY_STRING = PASS.

### Plain-permalink corroboration (diagnostic, same host)

After clearing `permalink_structure`:

- `index.php?p=` → 200 + POST_A
- `/wp-admin/index.php` → 200
- `/wp-admin/` → still 403 Access denied
- large `page_id` → 200 (~186 KiB)
- pretty URLs remain unusable without product front-controller

## Product gaps forcing P6E-C

These are reachable under a normal WordPress operator configuration and block
P6E-A / normal migration path. `PRODUCT_MUTATION=NO_BY_DEFAULT` → stop for owner.

1. **No CFD front-controller / try_files / PATH_INFO** — pretty permalinks map `SCRIPT_FILENAME` to a non-file path → FPM `File not found`.
2. **Directory URL → PHP script mapping missing** — `/wp-admin/` does not select `wp-admin/index.php`.
3. **FCGI request body cap 64 KiB** — media upload out of supported scope (explicit, not silently raised).
4. **No native Static BackendKind** — coexistence proven only via Proxy sidecar.

No WordPress-specific product fast path was added. P6MAXHONEST-A / ADR-042 preserved.

## Invalid / discarded runs

| RUN_ID | Reason |
| --- | --- |
| 20260831T142105Z | Host without port → self-301; `set -e` abort before completion |
| 20260831T142326Z | DB user password drift after DROP DATABASE |

## Migration value

```text
DOES_WORDPRESS_QUALIFICATION_INCREASE_MIGRATION_VALUE = PARTIAL
```

Real PHP app + DB + FPM path works for **plain** WordPress (`/`, `index.php`, login POST, REST query, reload, fail-closed max_conn).
Normal operator WordPress (pretty permalinks, `/wp-admin/`) does **not** qualify on CURRENT CFD without product work.

## Recommended next (NOT authorized)

```text
RECOMMENDED_NEXT_WIP =
  CFD_FRONT_CONTROLLER_OR_DIRECTORY_INDEX_MAPPING_ADR
  OR PLAIN_PERMALINK_ONLY_WORDPRESS_OPERATOR_CONTRACT_NARROWING
NEXT_WIP_AUTHORIZED = NO
```

Do not start async FastCGI, Phase 7, or rival benchmarks from this terminal.

## Independent audits

Specialist review on terminal HEAD (no product mutation). Full ledger:
`.exyonq-local/evidence/phase6e-current-head-real-wordpress-qualification/20260831T142351Z/AUDITS.md`.

| Agent | Verdict |
| --- | --- |
| exyonq-orchestrator | CONDITIONAL ACCEPT |
| exyonq-vertex-architect | CONDITIONAL ACCEPT |
| exyonq-security-engineer | CONDITIONAL ACCEPT |
| exyonq-grc-engineer | PASS (P6E-C terminal close) |
| exyonq-logic-auditor | PASS_WITH_NONBLOCKING_P3 |

### P6E-B vs P6E-C

Vertex: missing try_files is **NOT_IMPLEMENTED outside current CFD contract** — P6E-B
eligible only if owner narrows operator contract to explicit `.php` paths.

GRC + Logic: owner question includes **normal WordPress migration** (pretty permalinks,
`/wp-admin/` directory). Those fail on attributable product gaps → **P6E-C**.

**Terminal CASE = P6E-C** stands.

### Harness oracle debt (nonblocking)

Logic auditor OPEN_LOGIC_P2=5, OPEN_LOGIC_P3=2 — harness false CGI FAIL, `curl -L`
under pretty, shard/concurrency labels, KEEP_CONN pcap, login cookie oracle, contamination
vacuity. Does **not** change CASE. Future WIP: `HARNESS_DEBT.txt` in VALID_RUN package.

## Evidence / harness / docs

```text
EVIDENCE = .exyonq-local/evidence/phase6e-current-head-real-wordpress-qualification/20260831T142351Z/
  # Includes AUDITS.md, SOURCE_IDENTITY.txt, HARNESS_DEBT.txt
HARNESS = scripts/reality/run-phase6e-wordpress-qualification-netcup.sh
REMOTE = scripts/remote/v044-phase6e-wordpress-qualification-netcup.sh
TERMINAL = docs/governance/v044-phase6e-current-head-real-wordpress-qualification-terminal-block.txt
```
