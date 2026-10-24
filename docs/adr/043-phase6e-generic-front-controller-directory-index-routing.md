# ADR 043 — CFD generic front-controller + directory-index routing

```text
ADR = 043
TITLE = CFD generic front-controller and directory-index routing
STATUS = ACCEPTED
DATE = 2026-08-31
WIP = V044_PHASE6E_WORDPRESS_ROUTING_REPAIR
PARENT_TERMINAL = P6E-C (IMMUTABLE historical)
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
DO_NOT_REUSE = ADR-040 | ADR-041 | ADR-042
ORCHESTRATOR = CONDITIONAL_ACCEPT
VERTEX_PRE_MUTATION = CONDITIONAL_ACCEPT
SECURITY_PRE_MUTATION = CONDITIONAL_ACCEPT
RESEARCH_PRE_MUTATION = CONDITIONAL
GRC_PRE_MUTATION = CONDITIONAL
PRODUCT_MUTATION_AUTHORIZED = YES_AFTER_THIS_ADR
WORDPRESS_SPECIFIC_PRODUCT_FAST_PATH = FORBIDDEN
ASYNC_FASTCGI = NO
MAX_CONN_CONTRACT_CHANGE = NO
PHASE_7 = NO
```

## Context

P6E-C (immutable) proved real WordPress 7.1 + PHP-FPM + MariaDB on Netcup amd64 under CFD
FastCGI (`max_conn=1`). Two **supported product** gaps remained:

1. **Pretty permalink / front-controller** — `/hello-world/` built
   `SCRIPT_FILENAME=<docroot>/hello-world/` (non-file) → FPM `File not found`.
   Plain `index.php?p=` worked.
2. **Directory index** — `/wp-admin/` sent a directory as `SCRIPT_FILENAME` → FPM
   `Access denied`. Explicit `/wp-admin/index.php` worked.

Core / `.htaccess` overlay already defines generic `DirectoryIndex` + front-controller
semantics (`module-api` validators, Plan 11 tests). CFD bypassed that stage:

```text
RouteTable::lookup → script_filename_under_root(document_root, request_path) → FPM
```

```text
DEFECT_1_ROOT_CAUSE = FRONT_CONTROLLER_STAGE_NOT_IN_CFD
DEFECT_2_ROOT_CAUSE = NO_DIRECTORY_INDEX_STAGE
  (+ DIRECTORY_PATH_SENT_AS_SCRIPT_FILENAME consequence)
```

## Decision

### Reuse, do not invent a third routing model

```text
NO_NEW_PATTERN_ALTERNATIVE = SUFFICIENT
```

Project the **same semantic contract** as Plan 11 / `htaccess_overlay` into CFD
generation. Do **not** call core/module runtime per request. Do **not** add
WordPress path branches.

### Chosen precedence (FastCGI-matched routes only)

```text
CHOSEN_ROUTING_PRECEDENCE =
  1. Cap033 host/path route lookup (unchanged; WAF before lookup unchanged)
  2. If BackendKind::Proxy | Reject → existing behavior (no FC/DI)
  3. If BackendKind::Fastcgi → resolve script BEFORE execute_get:
     a. existing regular file ending .php → that script (explicit PHP)
     b. existing regular non-.php file → do not FastCGI-execute
        (static ownership remains more-specific Proxy routes / Cap067;
         on FastCGI route → controlled 404, never source disclosure)
     c. existing directory (URI ends with `/`, or path is a directory) →
        configured DirectoryIndex candidates in operator order;
        first existing .php → that script;
        first existing non-.php → not served as FastCGI (404 on this path)
     d. path does not exist as file or directory + configured front controller →
        configured target .php under docroot (require_not_file/dir already true)
     e. else → CFD 404 (genuine miss; no arbitrary PHP)
  4. execute_get with SCRIPT_FILENAME from resolved script;
     REQUEST_URI = original client request target (query preserved)
```

### Config SSOT

```text
ROUTING_CONFIG_SSOT = CFD control-plane text projection → RouteTable generation
DIRECTORY_INDEX_SSOT = basename-only candidates (same rules as
  module-api validate_directory_index_candidate); compiled onto CompiledFcgiPool
FRONT_CONTROLLER_SSOT = literal local target URI (same rules as
  validate_front_controller_target); CompiledFrontControllerPolicy on pool
SCRIPT_RESOLUTION_SSOT = CFD dataplane resolver invoked before execute_get;
  never FPM error as control flow for routing
```

Control-plane lines (ADR-043):

```text
fcgi|…|total_ms                 # existing (ADR-041/042 unchanged)
fcgi-dir-index|pool_id|index.php[|index.html…]
fcgi-front-controller|pool_id|/index.php
```

Empty / absent policy → prior behavior (no DI/FC). No implicit `index.php` on every miss.

### Generation schema

```text
GENERATION_SCHEMA_CHANGE_REQUIRED = YES
PAYLOAD_MAGIC = CFDRT004
```

CFDRT003 continues to decode with empty directory-index and no front-controller.
CFDRT004 appends per-pool: directory-index candidate list + optional front-controller
policy (`target_uri`, `require_not_file`, `require_not_directory`, `preserve_query`).

### CGI / PATH_INFO

```text
WORDPRESS_PATH_INFO_REQUIRED = NO
```

Research (RFC3875 + WordPress `parse_request` + conventional Apache rewrite): pretty
permalinks need `SCRIPT_FILENAME=<docroot>/index.php`, `SCRIPT_NAME=/index.php`, and
**preserved** original `REQUEST_URI` / `QUERY_STRING`. Do not synthesize PATH_INFO.

### Security invariants

```text
CLIENT_CONTROLS_SCRIPT_FILENAME = NO
STATIC_MISS != EXECUTE_ARBITRARY_PHP
DOCROOT_ESCAPE = 0          # canonicalize existing targets; reject outside root
PHP_SOURCE_DISCLOSURE = 0   # never FastCGI-execute non-.php as script source dump
INDEX = basename-only, no slash/.. /NUL /\\ /%
FRONT_CONTROLLER_TARGET = eager shape validation + request-time existence/canonical
```

### Cap067 / performance

```text
CAP067_INTERACTION = PASS
```

Repair is FastCGI-route-only. No native Static BackendKind, no Cap067/sendfile changes,
no global hot lock, no per-request control RPC, no Tokio/Hyper on CFD FastCGI path,
ADR-042 `max_conn=1` preserved.

Expected cost on FC/DI FastCGI requests only: small metadata probes (typically 1–N
`stat`/`canonicalize`); zero new locks.

### Alternatives

| ID | Option | Verdict |
|----|--------|---------|
| A | CFD-native compiled DI/FC before `execute_get` | ACCEPT (selected with B semantics) |
| B | Project htaccess/overlay DI/FC subset into CFD generation | ACCEPT PREFERRED semantics |
| C | Call core htaccess runtime from CFD per request | REJECT (process isolation) |
| D | Filesystem fallback inside `execute_get` only | REJECT (wrong layer) |
| E | WordPress-specific path logic | REJECT |
| F | Async / multi-inflight FastCGI | REJECT (out of scope) |

## Consequences

- WordPress pretty permalinks and `/wp-admin/` work **because** generic DI/FC is correct.
- Operators must configure `fcgi-dir-index` / `fcgi-front-controller` explicitly.
- P6E-C remains historically true (`MUST_FIX` at that terminal).
- Media upload 64 KiB body cap unchanged (`NOT_IN_CURRENT_SUPPORTED_SCOPE`).

## Rollback

Revert CFDRT004 fields and resolver; republish CFDRT003-compatible generations.
No Cap067 / pool / max_conn rollback required if untouched.

```text
ADR_CREATED_BEFORE_PRODUCT_MUTATION = YES
PRODUCT_MUTATION_AUTHORIZED = YES_BOUNDED_AFTER_THIS_ADR
```
