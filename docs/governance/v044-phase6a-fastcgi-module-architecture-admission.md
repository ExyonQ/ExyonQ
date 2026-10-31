# V044 Phase 6A — FastCGI / Module architecture admission

```text
WIP                          = V044_PHASE6A_FASTCGI_MODULE_ARCHITECTURE_ADMISSION
PHASE                        = 6A
MODE                         = READ_ONLY_ARCHITECTURE_SOURCE_RUNTIME_ADMISSION
STATUS                       = COMPLETE
CASE                         = P6A-C
PRODUCT_MUTATION             = NO
ARCHITECTURE_MUTATION        = NO
FASTCGI_MUTATION             = NO
MODULE_API_MUTATION          = NO
PHASE_6B_AUTHORIZED          = NO
IMPLEMENTATION_CLAIMED       = NO
ENTRY_HEAD                   = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE                   = ac059750141ee48f141c543cccbd641652bf49fb
BRANCH                       = release/v0.4.4-integration
TERMINAL_UTC                 = 2026-08-30T10:24:40Z
EVIDENCE                     = .exyonq-local/evidence/phase6a-fastcgi-module-architecture-admission/
NOTION_RECONCILIATION_REQUIRED = YES
```

## 0. Authority and freeze

Owner-authorized **admission only**. No FastCGI/Module product code change. No Phase 6B start.

```text
CURRENT_WORKTREE     = /Volumes/Lexar/Cursor/exyonq-laboratorio
CANONICAL_MERGE      = NO
PUSH                 = NO
TAG                  = NO
RELEASE              = NO
PUBLICATION          = NO
STOP_AFTER_TERMINAL  = YES
WIP_LIMIT            = 1
```

Ambient dirty tree exists (CFD WIP + other); Phase 6A attributes **no** product mutation to this WIP.

### Required agents

| Agent | Path | Status |
| --- | --- | --- |
| exyonq-orchestrator | `.cursor/agents/exyonq-orchestrator.md` | PRESENT |
| exyonq-vertex-architect | `.cursor/agents/exyonq-vertex-architect.md` | PRESENT |
| exyonq-security-engineer | `.cursor/agents/exyonq-security-engineer.md` | PRESENT |
| exyonq-research-engineer | `.cursor/agents/exyonq-research-engineer.md` | PRESENT |
| exyonq-grc-engineer | `.cursor/agents/exyonq-grc-engineer.md` | PRESENT |
| exyonq-logic-auditor | `.cursor/agents/exyonq-logic-auditor.md` | PRESENT (independent) |

```text
REQUIRED_AGENT_SET_STATUS = PASS
SILENT_SUBSTITUTION       = FORBIDDEN
```

### Binding competitive invariants (preserved)

```text
PROXY_DP_ISOLATION                              = SEPARATE_PROCESS
TARGET_PROCESS_MODEL                            = HYBRID
COMPETITIVE_H1_ACCEPT_OWNER                     = DATAPLANE_PROCESS
TOKIO_THREADS_IN_COMPETITIVE_DATAPLANE          = 0
HYPER_ON_COMPETITIVE_H1_PATH                    = 0
CONTROL_PLANE_RPC_PER_REQUEST                   = 0
CROSS_PROCESS_RPC_PER_REQUEST                   = 0
DUAL_ACCEPT_COMPETITIVE_H1                      = FORBIDDEN
PER_REQUEST_ACCEPT_HANDOFF                      = FORBIDDEN
TRANSPARENT_RUNTIME_FALLBACK_AFTER_DP_SIDE_EFFECT = FORBIDDEN
NEW_GLOBAL_HOT_LOCK                             = FORBIDDEN
```

---

## 1. Primary answers (§4)

| # | Question | Answer |
| --- | --- | --- |
| 1 | What FastCGI exists today? | Full production crate `crates/exyonq-mod-fastcgi`: wire codec, PARAMS, STDIN, KEEP_CONN `ConnPool`, generation drain, `CommitStage` safe-retry, Unix+TCP, CLI→core registration, cache hook surface. |
| 2 | What Module API exists today? | `module-api`: async `Module` trait (`on_init`/`on_request`/`on_route`/`on_upstream`/`on_response`/`on_shutdown`), registry, `fcgi_dispatch` contracts, Hyper `HttpRequest`/`HttpResponse`, addon-api re-exports. Prefer addon-api 1.x; Module path is legacy-compatible. |
| 3 | Source-reachable? | **YES** — FastCGI + Module API compiled into product CLI/core path. |
| 4 | Real runtime evidence? | **HISTORICAL** Cap008/009/038 dual-Linux (2026-08-07/11) ≠ current HEAD. **NO_CURRENT_PROOF** on `d9485666`. |
| 5 | Legacy Tokio/Hyper assumptions? | **YES** — `spawn_blocking`, Tokio semaphore/timeouts, Hyper bodies/types on product FastCGI+Module path. |
| 6 | Enter separate-process dataplane without invariant break? | **Codec/pool logic: only after sync re-host.** Current runtime/Module shell: **NO**. CFD dataplane FastCGI symbols today: **0**. |
| 7 | Adapter required? | **YES** for any CFD path: immutable generation IR + fail-closed backend kind; **not** Tokio Module adapter into CFD. |
| 8 | New dataplane implementation? | **YES** if owner insists FastCGI execute inside competitive H1 process. |
| 9 | Control-plane only? | Module lifecycle, registry, generation compile, reload/drain orchestration, observability export. FastCGI **execution** today is product/core (Tokio), not CFD. |
| 10 | Defer? | Full Module extensibility, WASM, WordPress CMS soak, arbitrary third-party native modules, CFD KEEP_CONN pool, FastCGI perf claims. |
| 11 | Smallest safe Phase 6B mutation? | See §43 — **not authorized here**. |

---

## 2. FastCGI source map

| Area | Location | Notes |
| --- | --- | --- |
| Crate | `crates/exyonq-mod-fastcgi` | Production wire via executor + ConnPool |
| Record/wire | `record.rs`, `encode.rs`, `parser.rs`, `wire.rs`, `fcgi_stream.rs` | FCGI_KEEP_CONN, PARAMS/STDIN/STDOUT/STDERR/END_REQUEST |
| Pool | `conn_pool.rs`, `pool.rs`, `pooled_forward.rs` | checkout/checkin/discard; generation; idle prune; one stale-idle safe retry |
| Commit | `commit.rs` | `CommitStage::NotStarted` only → `allows_safe_retry` |
| Runtime | `runtime.rs` | Tokio `spawn_blocking` + semaphore + delegate timeout |
| Params | `params.rs` | CGI PARAMS builder; SCRIPT_FILENAME from resolver authority |
| Adapter/bridge | `adapter.rs`, `bridge.rs` | Core registration (`core-bridge`) |
| Caps | `caps.rs` | MAX_* stdin/response/params/records/timeouts |
| Script resolve | `script_resolver.rs` + `module-api` fcgi script resolver | DOCUMENT_ROOT / path safety |
| Cache | `cache_serve.rs`, `cache_metrics.rs` | Hyper response types on product path |
| Core seam | `core/src/execute_backend.rs` | `FcgiDispatchService`; absent → **501** |
| CLI | `cli/exyonq/src/main.rs` | Registers production pools + cache hooks |
| CFD | `crates/exyonq-cfd-dataplane` | **No FastCGI**; proxy hop-by-hop only |
| CFD budget | `crates/exyonq-cfd-control/src/budget.rs` | `fastcgi_budget=4` **documentary headroom only** |

### Reachability classification (not synonyms)

| Component | SOURCE_EXISTS | SOURCE_REACHABLE | ROUTING_REACHABLE | CURRENT_PRODUCT_REACHABLE | REAL_RUNTIME_PROVEN | CURRENT_SHIPPING_ADMISSIBLE |
| --- | --- | --- | --- | --- | --- | --- |
| Wire codec | YES | YES | YES (product) | YES (when registered) | HISTORICAL | CONDITIONAL |
| ConnPool KEEP_CONN | YES | YES | YES (product) | YES | HISTORICAL | CONDITIONAL |
| Safe retry | YES | YES | YES | YES | UNIT+HISTORICAL | YES (contract) |
| CLI registration | YES | YES | YES | YES | HISTORICAL | YES |
| Unregistered 501 | YES | YES | YES | YES | UNIT | YES |
| CFD FastCGI execute | NO | NO | NO | NO | NONE | NO |
| Module hooks on CFD | NO | NO | NO | NO | NONE | NO |

---

## 3. Current FastCGI call graph (product path)

```text
accept (product Hyper/Tokio listener)
  → handler / wire_dispatch
  → module pipeline (on_request / on_route)     [product only]
  → route → Backend::Fastcgi
  → execute_backend::execute_fastcgi_backend
  → FcgiDispatchService::dispatch (registered runtime)
  → spawn_blocking → ConnPool::forward_once_with_budget
       checkout → BEGIN/PARAMS/STDIN → read STDOUT/STDERR/END_REQUEST
       checkin iff clean Ok + stage gate; else discard
  → map CGI status/headers/body → client
  → on_response filters
```

| FILE | TYPE | FUNCTION | CALLER | CALLEE |
| --- | --- | --- | --- | --- |
| `cli/exyonq/src/main.rs` | composition | register FastCGI | startup | `exyonq_mod_fastcgi` + `register_fcgi_*` |
| `core/src/execute_backend.rs` | dispatch | `execute_backend(Fastcgi)` | handler | `FcgiDispatchService` or 501 |
| `…/runtime.rs` | runtime | `run_delegate` / dispatch | service | `spawn_blocking` → pool forward |
| `…/conn_pool.rs` | pool | `forward_once_with_budget` | runtime | checkout/forward/checkin\|discard |
| `…/commit.rs` | contract | `allows_safe_retry` | pool | NotStarted only |
| `…/pooled_forward.rs` | wire | forward + parse | pool | encode/parser |
| CFD `shard.rs` | dataplane | proxy relay | CFD accept | **no FastCGI callee** |

---

## 4. Legacy dependency classification

| Dependency | Class |
| --- | --- |
| Tokio runtime / `spawn_blocking` / semaphore / timeout | **DATAPLANE_BLOCKER** for competitive H1 process |
| Hyper Request/Response / body types (Module + cache_serve) | **DATAPLANE_BLOCKER** on competitive H1 path |
| `async_trait` Module hooks | **DATAPLANE_BLOCKER** (async shell) |
| std blocking Unix/TCP I/O in ConnPool/wire | **ADAPTER_REQUIRED** / reusable under sync re-host |
| `fcgi_dispatch` value contracts (`FcgiCompiledSlot`, outcomes) | **CONTROL_PLANE_ONLY** / IR-portable |
| Generation bind/drain APIs | **CONTROL_PLANE_ONLY** |
| Scripted peers / unit transports | **NOT_ON_REQUEST_PATH** (tests) |

```text
FASTCGI_LEGACY_TOKIO_DEPENDENCY  = YES_ON_PRODUCT_PATH
FASTCGI_LEGACY_HYPER_DEPENDENCY  = YES_INCIDENTAL_ON_PRODUCT_PATH
MODULE_LEGACY_TOKIO_DEPENDENCY   = YES
MODULE_LEGACY_HYPER_DEPENDENCY   = YES
```

---

## 5. Architecture classification

| Component | Class |
| --- | --- |
| FastCGI record codec / PARAMS encode / response parse | DIRECTLY_PORTABLE (logic) → needs sync shell |
| ConnPool KEEP_CONN + generation + discard rules | THIN_ADAPTER_REQUIRED → shard-local redesign |
| `FcgiRuntime` Tokio wrapper | DATAPLANE_REIMPLEMENTATION_REQUIRED |
| Module `async` hooks | DATAPLANE_REIMPLEMENTATION_REQUIRED / CONTROL_PLANE_ONLY |
| CFD FastCGI execute (today) | absent → DATAPLANE_REIMPLEMENTATION_REQUIRED |
| Observability appenders / reload | CONTROL_PLANE_ONLY |
| WASM plugins | DEFER (`WASM_REQUIRED_FOR_PHASE6 = NO`) |

```text
FASTCGI_ARCHITECTURE_CLASS   = DATAPLANE_REIMPLEMENTATION_REQUIRED
  # for competitive-dataplane execution; product-plane path remains CONTROL/PRODUCT
MODULE_API_ARCHITECTURE_CLASS = CONTROL_PLANE_ONLY
  # for Phase 6B minimum; competitive DP hooks = INCOMPATIBLE without new design
```

---

## 6. Pool, retry, body, framing

### Pool (independent of H1 upstream pool)

```text
KEEP_CONN                    = YES (FCGI_KEEP_CONN on forward)
CHECKIN                      = only clean Ok path with stage gate
DISCARD                      = errors, timeouts, abort gate, generation mismatch, drain
SAFE_STALE_IDLE_RETRY        = max 1, NotStarted only, connection-class errors
GENERATION                   = pool generation; reuse across generations = NO (runtime docs)
FASTCGI_POOL_CURRENTLY_SAFE  = CONDITIONAL
  # sound on product contracts; trailing-buffer checkin nuance = security SG-FCGI-05;
  # NOT proven under competitive DP; NOT rebound to current HEAD Linux
```

### Retry

```text
RETRY                        = ONLY_IF_NOT_STARTED
MAX_RETRY                    = 1
COMMIT_POINT                 = first BEGIN_REQUEST byte (BeginCommitted)
FASTCGI_RETRY_CONTRACT       = PASS  # source + unit; product path
```

### Body / backpressure

Product path materializes request body into dispatch (`Vec` / Full body) with caps (`MAX_FCGI_STDIN_BYTES`, response caps). Competitive DP Phase-4 bounded streaming **must not** silently inherit full buffering without an explicit Phase 6 contract.

```text
FASTCGI_BODY_BACKPRESSURE_COMPATIBILITY = UNKNOWN_FOR_CFD
  # product: bounded buffers + caps; CFD streaming contract not yet defined for FastCGI
```

### Framing

END_REQUEST required for success path; STDERR capped; malformed → discard/error mapping (502/504/503 classes). Reuse depends on complete protocol state on Ok path.

---

## 7. Security admission (peer: CONDITIONAL_ACCEPT)

```text
MODULE_SECURITY_MODEL              = DE_FACTO_FIRST_PARTY_TRUSTED_ONLY
MODULE_CAPABILITY_BOUNDARY_STATUS  = ABSENT_FOR_ARBITRARY_NATIVE
NATIVE_MODULE_TRUST_MODEL          = FIRST_PARTY_TRUSTED_ONLY
ARBITRARY_THIRD_PARTY_NATIVE_IN_DP = FORBIDDEN_UNTIL_BOUNDARIES
```

### Security gap ledger (selected)

| ID | THREAT | CURRENT | GAP | REACHABILITY | MUST_FIX | PHASE6B_BLOCKER |
| --- | --- | --- | --- | --- | --- | --- |
| SG-FCGI-04 | CGI CL vs body mismatch | hop-by-hop strip | no CL↔body check | High if hostile FPM | YES | YES (ship path) |
| SG-FCGI-05 | trailing bytes after END_REQUEST | discard on error; idle probe | silent drop then checkin | Medium | YES | YES |
| SG-FCGI-06 | Unix socket peer trust | path connect | no peercred | High multi-tenant | YES | CONDITIONAL |
| SG-MOD-01 | `on_route` before WAF | first-party allowlist | WAF bypass if third-party | High if opened | YES | YES for 3P |
| SG-MOD-03 | native modules in competitive DP | composition root | no capability sandbox | N/A today | YES (policy freeze) | YES |

```text
FASTCGI_SECURITY_STATUS = CONDITIONAL
  # SCRIPT_FILENAME authority resolver-owned; client cannot set via HTTP_* — PASS-class
  # framing/reuse + socket trust gaps remain MUST_FIX before new FastCGI admission ship
```

**No repair in 6A.** Supported reachable defects remain MUST_FIX for later tranches.

---

## 8. Reality / WordPress

| Claim | Grade |
| --- | --- |
| Cap008 real php-fpm dual-Linux | HISTORICAL_ONLY (2026-08-07; HEAD ≠ current) |
| Cap009 php/wordpress profiles | HISTORICAL_ONLY; wordpress = **fixture profile**, not full CMS |
| Cap038 pooling | HISTORICAL_ONLY (2026-08-11) |
| Current HEAD dual-Linux FastCGI | NO_CURRENT_PROOF |
| Darwin FastCGI product proof | NO_CURRENT_PROOF (Linux-required harnesses) |
| Full WordPress soak p1.2 | NO_CURRENT_PROOF (evidence/compose gaps) |

```text
WORDPRESS_CURRENT_REUSE_OF_HISTORICAL_EVIDENCE = VALID_CONTEXT_ONLY
FASTCGI_PERFORMANCE_CURRENTLY_PROVEN           = NO
PROFILE_A_FRONTIER_PROVES_FASTCGI_PERF         = NO
```

### Test trust

| Gate | FastCGI | Module API |
| --- | --- | --- |
| ZERO_FAKE harness design | PASS (Cap008/009 ban stubs) | N/A product hooks |
| Current ZERO_FAKE readiness | FAIL (no current HEAD package) | FAIL for DP hooks |
| NO_SMOKE harness design | PASS | PASS |
| Current NO_SMOKE readiness | FAIL (missing current proof) | FAIL for competitive DP |

```text
FASTCGI_ZERO_FAKE_READINESS = NO
FASTCGI_NO_SMOKE_READINESS  = NO
MODULE_ZERO_FAKE_READINESS  = NO
MODULE_NO_SMOKE_READINESS   = NO
```

Scripted peers are confined to unit/component tests; CLI production registration uses real pools (SOURCE_PROOF).

---

## 9. Module API inventory & hot path

Hooks: `on_init`, `on_request`, `on_route`, `on_upstream`, `on_response`, `on_shutdown`.

| Hook execution today | Plane |
| --- | --- |
| Product Hyper handler + module-pipeline | LEGACY_REQUEST_PROCESS / PRODUCT |
| Competitive CFD dataplane | NOT_CURRENTLY_REACHABLE |

Authority (first-party Module): mutate request view; `on_route` short-circuit; mutate response observation; process FS; panic can take host. **Not** a sandbox.

Hot-path cost (static): dynamic dispatch + async + Hyper types + possible clones → **HIGH** if forced onto competitive H1; **MEDIUM** on product path.

```text
MODULE_HOT_PATH_RISK               = HIGH_IF_ON_COMPETITIVE_DP
WASM_REQUIRED_FOR_PHASE6           = NO
```

### Intended order (freeze for Phase 6 design)

```text
accept → WAF/admit → request filters (modules) → route/overlay
  → FastCGI selection → PARAMS/STDIN → backend
  → response map → response filters → observability (async off hot path)
```

Competitive DP today: WAF/generation/proxy only — FastCGI/module filters **not** inserted.

---

## 10. Split-plane data contract (proposal only — not implemented)

Prefer compiled immutable generation IR published once per reload:

```text
CompiledFcgiPool {
  pool_id, transport (unix|tcp), address,
  document_root, max_connections, idle_timeout_ms,
  connect/read/write/total timeouts, generation
}
CompiledRoute.backend =
  Proxy { upstream_id } | Fastcgi { pool_id } | Static { … } | UnsupportedReject
```

```text
PER_REQUEST_IPC_REQUIRED                     = NO_REQUIRED
DATAPLANE_GENERATION_SCHEMA_CHANGE_REQUIRED  = YES  # if CFD must classify FastCGI
NEW_GLOBAL_HOT_LOCK_REQUIRED                 = NO_REQUIRED
TOKIO_IN_COMPETITIVE_DATAPLANE_REQUIRED      = NO_REQUIRED
HYPER_IN_COMPETITIVE_DATAPLANE_REQUIRED      = NO_REQUIRED
```

**No dual execution:** one accept owner per listen; FastCGI execute either product process **or** CFD — never both for the same production socket. Unsupported FastCGI on competitive listen → **fail closed** (not transparent fallback).

---

## 11. Ledgers

### Architecture gaps

| ID | CAPABILITY | CURRENT | TARGET | GAP | IMPACT | REQUIRED_MUTATION | BLOCKS_6B | EVIDENCE |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AG-01 | FastCGI in competitive DP | absent | sync execute or explicit out-of-scope | full | migration | CFD reimplement or IR reject | YES if 6B=DP execute | CFD grep=0 |
| AG-02 | Module hooks in DP | absent | defer or sync narrow hooks | design | extensibility | new contract | NO for min 6B | module-pipeline product-only |
| AG-03 | Generation FastCGI kind | proxy upstream_id only | Fastcgi\|Reject | schema | mis-route risk | gen IR | YES for safe CFD | cfd-gen RouteTable |
| AG-04 | Tokio FastCGI runtime | product | must not enter CFD | invariant | blocker | remove/replace shell | YES for DP port | runtime.rs |

### Reality gaps

| ID | CLAIM | SOURCE | REACHABLE | CURRENT_RUNTIME_PROOF | HISTORICAL_ONLY | REQUAL_REQUIRED |
| --- | --- | --- | --- | --- | --- | --- |
| RG-01 | Real PHP-FPM E2E | Cap008 scripts | YES | NO | YES | YES |
| RG-02 | WordPress product | Cap009 fixture / p1.2 | PARTIAL | NO | YES/MISSING | YES |
| RG-03 | FastCGI perf frontier | none | N/A | NO | N/A | YES (later) |

### Performance risks (no benchmark)

| Risk | Class |
| --- | --- |
| Tokio/Hyper re-entry into CFD | HIGH |
| Per-request IPC/RPC | HIGH (forbidden) |
| Full body buffer vs streaming | MEDIUM |
| Pool Mutex contention | MEDIUM |
| Module dynamic dispatch on H1 | HIGH |
| PHP-FPM connect churn | MEDIUM |
| Extra process hop | LOW if exclusive listen split |

```text
PERFORMANCE_RISK_COUNT = 7
```

---

## 12. Migration value

FastCGI + PHP-FPM is high-value for administrators migrating from NGINX/Apache PHP estates (unix socket pools, SCRIPT_FILENAME, WordPress front controller). Competitive Profile-A does **not** replace that value.

```text
DOES_PHASE6_HAVE_HIGH_MIGRATION_VALUE = YES
MIGRATION_VALUE_REASON =
  PHP-FPM/WordPress estate parity is a primary adoption surface;
  must be admitted without eroding competitive H1 proxy invariants.
```

---

## 13. Phase 6 sequencing (gated; each tranche needs new owner auth)

```text
P6A = architecture/reality admission          ← THIS WIP (COMPLETE)
P6B = smallest safe mutation (see below)      ← NOT AUTHORIZED
P6C = body/backpressure + safe pooling harden
P6D = narrow first-party module hooks (if any)
P6E = real PHP-FPM + WordPress requal (dual Linux)
P6F = FastCGI performance/frontier (separate from Profile-A)
```

### Smallest Phase 6B proposal (recommendation only)

**Preferred minimum (honest vs owner “dataplane admission” intent):**

```text
PROPOSED_PHASE6B_SCOPE =
  MINIMAL_FAILCLOSED_FASTCGI_IR_ON_CFD
  + PRODUCT_PLANE_FASTCGI_PRESERVE
  OR (larger) SYNC_CFD_NATIVE_FASTCGI_WIRE_MVP

MINIMUM_IF_PRESERVE =
  1) generation backend_kind Fastcgi|Reject on CFD routes
  2) competitive listen: FastCGI match → fail-closed (no FPM I/O, no Tokio)
  3) product process remains sole FastCGI execute owner on its listeners
  4) no Module extensibility; no WordPress; no dual accept
  5) security MUST_FIX checklist scheduled before claiming ship-ready FastCGI
  6) evidence plan: Cap008-class requal on then-current HEAD (Netcup+Oracle)

MINIMUM_IF_TRUE_DP_EXECUTE =
  sync codec+pool reimplementation inside CFD (no Tokio/Hyper/Module),
  GET+headers+bounded body+errors+generation+obs — separate ADR; larger than IR-only
```

```text
PHASE_6B_FEASIBLE     = CONDITIONAL
PHASE_6B_RECOMMENDED  = YES   # scoped; owner chooses preserve vs sync-DP MVP
PHASE_6B_AUTHORIZED   = NO
```

---

## 14. CASE selection

Owner CASE taxonomy:

| Case | Meaning |
| --- | --- |
| P6A-A | Ready for minimal Phase 6B implementation |
| P6A-B | Ready with nonblocking qualifications |
| P6A-C | Blocking product or architecture gaps |
| P6A-D | Current FastCGI/Module architecture incompatible |
| P6A-E | Insufficient evidence |

**CASE = P6A-C**

Reason: owner-preferred Phase 6B shape (`MINIMAL_REAL_FASTCGI_DATAPLANE_ADMISSION` into competitive architecture) collides with **DATAPLANE_REIMPLEMENTATION_REQUIRED** (Tokio/Hyper/Module shell; CFD FastCGI absent; gen schema lacks FastCGI kind). Product FastCGI contracts themselves are largely sound (logic PASS; retry PASS) — incompatibility is **with competitive DP invariants**, not “delete FastCGI.” Not P6A-D (architecture is preservable on product plane). Not P6A-A/B for **dataplane execute** 6B without closing AG-01/AG-03/AG-04.

If owner later redefines 6B as **product-preserve + CFD fail-closed IR only**, CASE may reopen as **P6A-B** under new authorization.

```text
OPEN_P0 = 0
OPEN_P1 = 0
OPEN_P2 = 0   # architecture gaps tracked as blockers to DP-6B, not as live product P0
OPEN_P3 = 1   # LA-P6A-001 checkin stage floor wording
SUPPORTED_CONTRACT_MUST_FIX_COUNT = 5  # security SG items; do not repair in 6A
ARCHITECTURE_GAP_COUNT = 4
SECURITY_GAP_COUNT     = 5
REALITY_GAP_COUNT      = 3
```

---

## 15. Performance budget note (§46–47)

Profile-A frontier metrics remain binding for **proxy** competitive H1. FastCGI erosion must be measured later on a **FastCGI-specific** campaign — never claim Profile-A as FastCGI proof.

```text
FASTCGI_PERFORMANCE_CURRENTLY_PROVEN = NO
```

---

## 16. Documentation / Notion

| Doc | Action |
| --- | --- |
| This file | CREATE (SoT for Phase 6A) |
| Architecture / roadmap / FastCGI / Module contracts | reconcile references only when touched; **no implementation begun claim** |

```text
VERSIONED_DOCS_RECONCILED      = PARTIAL  # SoT created; index row below
NOTION_RECONCILIATION_REQUIRED = YES
```

---

## 17. Terminal statement

Phase 6 is **not** “bring the old FastCGI code into CFD.” Admit only surfaces that preserve competitive invariants. Historical Cap008/009 are context, not current authority.

```text
REALITY_FIRST       = YES
ARCHITECTURE_FIRST  = YES
SECURITY_FIRST      = YES
MIGRATION_VALUE_REQUIRED = YES
PHASE_6B_AUTHORIZED = NO
STOP                = YES
```
