# V044 Phase 6B — Sync CFD-native FastCGI MVP

```text
WIP                          = V044_PHASE6B_SYNC_CFD_NATIVE_FASTCGI_MVP
PARENT                       = V044_PHASE6A_FASTCGI_MODULE_ARCHITECTURE_ADMISSION
PARENT_CASE                  = P6A-C
STATUS                       = TERMINAL_SEALED_P6BSEAL-B
ADR                          = 038 (owner template name "ADR-021 Phase6B"; ADR-021 = H1 reuse)
CHOSEN_ARCHITECTURE          = SYNC_CFD_NATIVE_FASTCGI_WIRE_MVP
PRODUCT_MUTATION             = YES_BOUNDED
MODULE_API_MUTATION          = NO
WORDPRESS                    = NO
WASM                         = NO
PHASE_6C_AUTHORIZED          = NO
ENTRY_HEAD                   = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE                   = ac059750141ee48f141c543cccbd641652bf49fb
TERMINAL_SEAL_DOC            = docs/governance/v044-phase6b-fastcgi-mvp-terminal-seal.md
SEAL_WIP                     = V044_PHASE6B_FASTCGI_MVP_TERMINAL_SEAL
RECONCILIATION_CLASS         = POSTHOC
PHASE6B_PRE_MUTATION_ARCHITECTURE_REVIEW = FAIL
PRE_MUTATION_PROOF           = MISSING
ADR021_GOVERNANCE_REQUIREMENT = INCOMPLETE
```

## Owner decision

Reject permanent CFD FastCGI-reject-only as Phase-6 end state.
Target: **real FastCGI execution in competitive CFD** with:

```text
TOKIO_IN_CFD = 0
HYPER_IN_CFD = 0
PER_REQUEST_IPC = 0
TRANSPARENT_FALLBACK = FORBIDDEN
DUAL_ACCEPT = FORBIDDEN
```

## ADR outcome

See [`docs/adr/038-phase6b-sync-cfd-native-fastcgi-mvp.md`](../adr/038-phase6b-sync-cfd-native-fastcgi-mvp.md).

Option **D** accepted; **B/C** rejected.

## Scope (MVP)

- `BackendKind::Fastcgi` in CFD generation IR (`CFDRT003`, schema version **2**)
- Sync wire crate `exyonq-fastcgi-wire`: BEGIN/PARAMS/STDIN/STDOUT/STDERR/END_REQUEST
- GET (+ optional small bounded CL body ≤ 64 KiB)
- Shard-local pool; KEEP_CONN only after clean END_REQUEST (SG-FCGI-05)
- Absolute `total_timeout_ms` enforced via `BudgetedStream`
- Real PHP-FPM proof on Netcup amd64
- No Module hooks, no WordPress, no FastCGI frontier bench

## Component classification

| Component | Class |
| --- | --- |
| Record codec / PARAMS / STDIN / STDOUT+STDERR / END_REQUEST | PORT_TO_RUNTIME_NEUTRAL (`exyonq-fastcgi-wire`) |
| CommitStage / NotStarted retry | REUSE (ported) |
| Caps / CGI parse | REUSE (ported) |
| Tokio ConnPool / spawn_blocking runtime | REIMPLEMENT_FOR_CFD (shard-local sync pool) |
| Module API | DEFER |
| WordPress | DEFER |

## Security contracts

| ID | Topic | P6B result |
| --- | --- | --- |
| SG-FCGI-04 | CL ↔ body ↔ STDIN | CLOSED (fail-closed mismatch) |
| SG-FCGI-05 | Trailing bytes after END_REQUEST | CLOSED (DISCARD; no checkin) |
| SG-FCGI-06 | Unix socket trust | QUALIFIED `SINGLE_TENANT_ADMIN_FS` (absolute path only) |

```text
UNIX_SOCKET_TRUST_MODEL = SINGLE_TENANT_ADMIN_FS
SCRIPT_FILENAME_AUTHORITY = trusted document_root + path (reject .. / NUL)
```

## Generation IR

```text
SCHEMA_VERSION = 2
ROUTE_MAGIC = CFDRT003
BackendKind = Proxy | Fastcgi | Reject
CompiledFcgiPool = { id, transport, document_root, timeouts, max_connections, … }
Legacy CFDRT002 = decode as Proxy-only
Incompatible schema = fail-closed at CFD start
```

## Real PHP-FPM evidence (Netcup amd64)

```text
PLATFORM = LINUX_AMD64 (netcup-bench)
RUN_ID = 20260830T114809Z  (post total_timeout fix replay)
PRIOR = 20260830T113234Z
EVIDENCE = .exyonq-local/evidence/phase6b-sync-cfd-native-fastcgi-mvp/<RUN_ID>/
REAL_PHP_FPM = PASS
REAL_DYNAMIC_PHP = PASS (counter + nonce)
REAL_CONCURRENT_REQUESTS = PASS
REAL_BACKEND_FAILURE = PASS (502 after FPM kill)
TOKIO_THREADS_IN_DATAPLANE_PROCESS = 0
HYPER_IN_DATAPLANE_PROCESS = 0
LEGACY_FASTCGI_RUNTIME_USED = NO_REQUIRED
```

## Claim limit

```text
CLAIM_STRENGTH_FREEZE = YES
P6B_MAY_CLAIM = MINIMAL_REAL_CFD_FASTCGI_MVP + CAUSAL_KEEP_CONN_NETCUP_AMD64
P6B_MUST_NOT_CLAIM = WordPress | FastCGI frontier | Module extensibility | full parity | shipping complete
  | pre-mutation architecture Accept | multi-tenant Unix peercred
ORACLE_ARM64_REQUIRED = NO for this tranche claim (platform-generic std sockets; dual-arch required before shipping admission of FastCGI as product capability)
FASTCGI_PERFORMANCE_FRONTIER = NOT_MEASURED
FASTCGI_SANITY_PERF = NOT_RUN
FASTCGI_KEEP_CONN_SEMANTICS = PASS
FASTCGI_CLEAN_REUSE_COMPONENT = PASS
FASTCGI_REAL_PHP_FPM_CAUSAL_REUSE = PASS
  # Netcup seal 20260830T142958Z: 1000 req / 1 SYN (tcpdump); not ExyonQ pool counters
ARCHITECTURE_AUDIT_MATERIAL_STATUS = DO_NOT_READ_AS_PRE_MUTATION_ACCEPT
SECURITY_AUDIT_CEILING = PASS_WITH_NONBLOCKING
LOGIC_AUDIT_CEILING = PASS_WITH_FINDINGS
ADR_NUMBER_AUTHORITY = 038
ADR021_PHASE6B_TEMPLATE_NAME = NON_AUTHORITATIVE
```

## Gates (workspace)

```text
CARGO_FMT_CHECK            = PASS
CARGO_CHECK_WORKSPACE      = PASS
CARGO_CHECK_FEATURE_MATRIX = PASS (default + http3-provider-quiche package runs)
CARGO_CLIPPY_CANONICAL     = PASS (workspace -D warnings + exyonq-wasm-host)
CARGO_TEST_WORKSPACE       = PASS (1483 passed; 0 failed)
```

First workspace run hit ambient flake `plan12_cache_concurrency_test::ttl_race_*` (unrelated to FastCGI); isolated recheck 3/3 PASS; full workspace rerun PASS.

## Terminal

```text
CASE                         = P6B-B
TERMINAL_HEAD                = d94856663f42ffce16a3c0ccc1291feb4d7083e4
TERMINAL_TREE                = fa77eedcbe854d7a2261aa49e1ba2b5144d4d6b2
  (uncommitted WIP; no Darwinian commit / PUSH)
TERMINAL_EVIDENCE_RUN        = 20260830T130030Z
ARCHITECTURE_AUDIT           = PASS
SECURITY_AUDIT               = PASS_WITH_NONBLOCKING
INDEPENDENT_LOGIC_AUDIT      = PASS_WITH_FINDINGS
PHASE_6C_AUTHORIZED          = NO
CANONICAL_MERGE              = NO
PUSH                         = NO
SEE_ALSO_TERMINAL_SEAL       = docs/governance/v044-phase6b-fastcgi-mvp-terminal-seal.md
SEAL_CASE                    = P6BSEAL-B
SEAL_EVIDENCE_RUN            = 20260830T142958Z
```

Historical P6B-B nonblocking note (preserved): KEEP_CONN causal reuse was not measured at P6B terminal; SG-FCGI-06 QUALIFIED; FCGI-RACE-01 checkout→BEGIN; `script_suffix` / `reuse_ok` residuals.

**Seal update:** causal KEEP_CONN measured PASS on Netcup (`20260830T142958Z`, 1000 req / 1 SYN). Remaining open P3: SG-FCGI-06, FCGI-RACE-01, script_suffix/reuse_ok. ADR pre-mutation analysis still INCOMPLETE (why seal is P6BSEAL-B, not A).

## Phase 6A history

Preserved in [`v044-phase6a-fastcgi-module-architecture-admission.md`](v044-phase6a-fastcgi-module-architecture-admission.md) (CASE P6A-C).
