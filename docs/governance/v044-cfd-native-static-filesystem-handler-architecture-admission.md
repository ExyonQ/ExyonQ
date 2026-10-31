# V044 — CFD native static filesystem handler — architecture admission

```text
WIP                          = V044_CFD_NATIVE_STATIC_FILESYSTEM_HANDLER_ARCHITECTURE_ADMISSION
MODE                         = READ_ONLY_ARCHITECTURE_ADMISSION
STATUS                       = COMPLETE
CASE                         = P7STATICADM-B
PARENT_TERMINAL              = P7-E (IMMUTABLE)
PRODUCT_MUTATION             = NO
ARCHITECTURE_MUTATION        = NO
IMPLEMENTATION               = NO
DOC_MUTATION                 = YES
HARNESS_MUTATION             = NO
TEST_MUTATION                = NO
BACKENDKIND_STATIC_CODE      = NOT_AUTHORIZED
PHASE_7_REQUALIFICATION      = NO
PHASE_8_AUTHORIZED           = NO
FASTCGI_ASYNC_AUTHORIZED     = NO
FASTCGI_PERFORMANCE_FRONTIER = NOT_AUTHORIZED
BROAD_P1_P14_BENCHMARK       = NOT_AUTHORIZED
ENTRY_HEAD                   = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE                   = ac059750141ee48f141c543cccbd641652bf49fb
BRANCH                       = release/v0.4.4-integration
STOP_AFTER_TERMINAL          = YES
EVIDENCE                     = .exyonq-local/evidence/cfd-native-static-filesystem-handler-architecture-admission/20260831T230825Z/
```

ADR: [`docs/adr/044-cfd-native-static-filesystem-handler.md`](../adr/044-cfd-native-static-filesystem-handler.md)  
Parent: [`v044-phase7-static-proxy-coexistence.md`](v044-phase7-static-proxy-coexistence.md) (**P7-E**)

---

## 0. Scope guard

```text
CURRENT_WORKTREE             = <repo>
AUTHORIZED_SCOPE             = architecture admission docs + ADR-as-docs + evidence under .exyonq-local/
FILES_ALLOWED_TO_CHANGE      =
  docs/governance/v044-cfd-native-static-filesystem-handler-architecture-admission.md
  docs/governance/v044-cfd-native-static-filesystem-handler-architecture-admission-terminal-block.txt
  docs/adr/044-cfd-native-static-filesystem-handler.md
  docs/governance/README.md
  docs/governance/v044-phase7-static-proxy-coexistence.md   # pointer / NEXT_WIP name only
  docs/governance/v044-fastcgi-application-compatibility-matrix.md  # native-static terminology only
OPERATIONS_FORBIDDEN         =
  COMMIT / PUSH / TAG / RELEASE / MERGE / SYNC_TO_GITHUB
  BackendKind::Static product mutation
  Cap067 / exyonq-mod-static / core Cap067 mutation
  Phase 8 / async FastCGI / competitive perf
```

Ambient dirty tree preserved (tracked product + untracked CFD). No cleanup / stash / reset.

### Entry freeze (ambient classification)

| Class | Tracked diff | Untracked |
| --- | --- | --- |
| product / Cargo | 31 | 10 |
| docs | 10 | 93 |
| harness / scripts | 2 | 51 |
| CFD crates (untracked) | — | 37 |
| porcelain lines | 192 | 191 untracked |

Source health (attributable to CFD packages at ENTRY_HEAD objects + ambient CFD tree):

```text
cargo fmt --check -p exyonq-cfd-gen -p exyonq-cfd-dataplane  → OK
cargo check -p exyonq-cfd-gen -p exyonq-cfd-dataplane        → OK
cargo test -p exyonq-cfd-gen                                 → 13 passed
```

---

## 1. Binding parent truth (P7-E / P6G)

```text
CURRENT_PHASE_TERMINAL                    = P7-E
P7_STATIC_GAP                             = CFD_NATIVE_STATIC_HANDLER_ABSENT
CURRENT_CFD_BACKEND_KIND                  = Proxy | Fastcgi | Reject
PROXY_FASTCGI_SAME_GENERATION             = PASS
STATIC_PROXY_FASTCGI_COEXISTENCE          = NOT_REACHABLE_WITH_CURRENT_PRODUCT
SUPPORTED_CONTRACT_MUST_FIX_COUNT         = 0
NATIVE_CFD_STATIC_HANDLER_CURRENTLY_EXISTS = NO
P6G_NATIVE_STATIC_CLAIM                   = NOT_PROVEN_AND_NOT_REQUIRED_FOR_P6G_A
P6G_A                                     = VALID (static coexistence evidence was Proxy→Python fileserver)
CAP067_STATUS                             = CLOSED_VERIFIED_REAL_PRODUCTION (product-plane)
SYMLINK_TOCTOU_STATUS                     = BOUNDED_RESIDUAL_PRESERVED
PHASE8_TOCTOU_REVIEW_REQUIRED             = YES
```

**Later event (does not rewrite CASE P7STATICADM-B):** owner later authorized Phase 8. Terminal **P8-C**: TOCTOU remains `NOT_REPRODUCED_RESIDUAL_PRESERVED` (`REVIEW_BEFORE_FINAL_SHIPPING_ADMISSION=YES`); new MUST_FIX `P8-STATIC-FIFO-001` (Linux `openat2` FIFO hang). See [v044-phase8-completeness-and-adversarial-security.md](v044-phase8-completeness-and-adversarial-security.md).

### Terminology (canonical)

Absence of native Static is **not** a discovered regression of a supported CFD Static contract.
It is a **required capability gap** for NGINX/Apache/OLS-like static + proxy + PHP coexistence.

### Terminology (canonical)

```text
NATIVE_STATIC = CFD opens and serves the filesystem object itself
NOT = CFD proxies to an HTTP file server
NOT = FastCGI returns file bytes
NOT = application returns static bytes
NOT = control plane serves the file
```

---

## 2. Current CFD request path map (source-bound)

```text
listener (mio shard accept)
  → request parse (H1 headers in shard.rs)
  → WAF (compiled gen projection)
  → RouteTable::lookup(path, host)   # Cap033: exact > wildcard > hostless; longest path prefix
  → BackendTarget match
       Proxy    → upstream connect + relay
       Fastcgi  → fcgi_route resolve (DI/FC) + sync FastCGI execute
       Reject   → 501
  → queue_response / client write (to_client_head + to_client buffers; WouldBlock park)
```

| Concern | SSOT |
| --- | --- |
| Route table / Cap033 | `crates/exyonq-cfd-gen/src/route_table.rs` (`RouteTable::lookup`, `CompiledRoute`) |
| Backend kind | `BackendKind::{Proxy=0, Fastcgi=1, Reject=2}` — **no Static** |
| Dispatch target | `BackendTarget::{Proxy, Fastcgi, Reject}` |
| Generation magic | `CFDRT004` (+ legacy `CFDRT003`/`CFDRT002`) |
| Publication | `exyonq-cfd-gen` encode + publish CLI / control |
| Worker access | shard holds `Arc`/gen snapshot; no per-request control RPC |
| Proxy exec | `shard.rs` upstream phases |
| FastCGI exec | `shard.rs::dispatch_fcgi` + `fcgi_route.rs` |
| Reject | `queue_error(501)` |

```text
CURRENT_CFD_REQUEST_PIPELINE     = listen→parse→WAF→Cap033 lookup→Proxy|Fastcgi|Reject→buffer write
CURRENT_HANDLER_DISPATCH_SSOT    = crates/exyonq-cfd-dataplane/src/shard.rs match BackendTarget
CURRENT_ROUTE_SSOT               = crates/exyonq-cfd-gen/src/route_table.rs CFDRT004
```

---

## 3. Current product-plane static map

Primary crate: `crates/exyonq-mod-static` (+ `module-api/src/static_wire.rs` hooks into core epoll/Tokio).

| Capability | Status in product plane |
| --- | --- |
| root / docroot | `StaticRoot` + canonicalize + Linux root dirfd |
| alias | not required for CFD MVP (see §16) |
| DirectoryIndex | product/FastCGI ADR-043; not Static listing |
| MIME | `content_type_for` extension table |
| HEAD | Cap067 sendfile FSM head-only |
| Range | Cap019 (`byte_range`, sendfile range) |
| If-Modified-Since / ETag | Cap020 validators / `PreparedStaticWire` |
| sendfile | `sendfile.rs` + `sendfile_fsm.rs` (Linux) |
| fallback read | bounded / wire body paths |
| openat2 RESOLVE_BENEATH | Cap004 in `sendfile.rs` / `StaticRoot` |
| FD reuse | `sendfile_fd_cache` worker-local Cap067 |
| metadata / identity | `FileIdentity` / `StaticResourceIdentity` |
| path normalize | `relative_as_safe_path` |
| symlink | in-docroot allowed if final under root; escape blocked; **TOCTOU residual** |
| 403/404 | miss / escape terminals |
| autoindex | not CFD MVP |
| Tokio/Hyper shell | `static_wire` hooks — **non-reusable in CFD** |

```text
CURRENT_PRODUCT_STATIC_CAPABILITIES =
  Cap004 containment + Cap067 sendfile FSM + Cap019/020 + MIME + root preload/cache
CURRENT_PRODUCT_STATIC_RUNTIME_DEPENDENCIES =
  exyonq-mod-static ↔ module-api static_wire ↔ core epoll/Tokio workers
CURRENT_PRODUCT_STATIC_REUSABLE_LOGIC =
  path normalize ideas; openat2 RESOLVE_BENEATH pattern; fstat-after-open identity;
  MIME extension table; sendfile(2) syscall semantics; REGULAR_FILES_ONLY policy
CURRENT_PRODUCT_STATIC_NON_REUSABLE_RUNTIME_SHELL =
  StaticWire* hooks; Tokio handoff; epoll_session product worker; linking whole crate into CFD
```

---

## 4. Cap067 deep review

```text
CAP067_STATUS = CLOSED_VERIFIED_REAL_PRODUCTION   # product-plane
CAP067_STATIC_PATH_ON_CFD = NOT_APPLICABLE
```

| Reuse | Must not copy blindly |
| --- | --- |
| Cap004 open-under-root / `openat2(RESOLVE_BENEATH)` + ENOSYS fallback | Entire `exyonq-mod-static` dependency graph / Tokio |
| Regular-file + fstat identity after open | Product `StaticRoot` preload HashMap as CFD gen default |
| Nonblocking sendfile pump / EAGAIN park *pattern* | Cap067 FD cache keys tied to product generation/site_slot |
| MSG_MORE header+body coalesce *idea* | Assuming `queue_response` can sendfile today (it cannot) |
| Avoid open→stat→close thrash *goal* | Porting worker-local FD cache before MVP proves need |

```text
WHAT_CAN_CFD_REUSE_FROM_CAP067 =
  portable containment + identity + sendfile/EAGAIN patterns (extract-or-reimplement)
WHAT_MUST_NOT_BE_COPIED_BLINDLY =
  crate link, wire hooks, product residency, FD cache without CFD generation keys
CAP067_STATIC_ARCHITECTURE_INTERACTION =
  Cap067 remains product EXYONQ_EPOLL_STATIC plane; CFD Static is a sibling capability
CAP067_FD_REUSE_IN_CFD =
  NOT_IN_MVP; optional later generation-scoped shard-local cache with (dev,ino) revalidation
```

Native CFD Static must **not** casually reintroduce per-request open/stat/close thrash without measuring; MVP may open+fstat+transfer+close per request with **no** FD cache, then add Cap067-class reuse only with identity oracle.

---

## 5. NO_NEW_PATTERN

Can current CFD support **native** static without a new handler category?

| Interpretation | Result |
| --- | --- |
| Reuse Proxy | Serves HTTP upstream — **not** NATIVE_STATIC |
| Reuse FastCGI | Application/script execution; DI/FC NotFound ≠ file serve (`fcgi_route.rs`) |
| Reuse Reject | Error only |
| Product Cap067 via per-request handoff | Forbidden (RPC / dual-accept / plane violation) |
| Compile static into FastCGI DI | Conflates Static vs front-controller; discloses wrong ownership |

```text
NO_NEW_PATTERN_RESULT = INSUFFICIENT
WHY = no existing BackendKind opens/serves filesystem objects inside CFD;
      Proxy-to-fileserver and FastCGI NotFound are not native Static
```

---

## 6. Architecture alternatives

| ID | Option | Verdict |
| --- | --- | --- |
| A | `BackendKind::Static` | **ACCEPT** — mirrors ADR-038 FastCGI IR extension |
| B | Rename to `HandlerKind`/`RouteAction` | Equivalent rename; unnecessary churn |
| C | Static policy outside BackendKind, same dispatch | Extra SSOT; higher ambiguity |
| D | Generic response-source executor | Over-abstract for three handlers |
| E | Proxy to local static sidecar | Rejected (not native; Kobayashi risk) |
| F | Per-request product-plane handoff | Rejected (RPC / dual-accept) |
| G | Main-process static via IPC | Rejected (cross-process per request) |
| H | No native CFD static; sidecar forever | Rejected as **final** architecture (ops gap remains) |
| I | NO_NEW_PATTERN | Insufficient (above) |

```text
CHOSEN_HANDLER_REPRESENTATION = BackendKind::Static (+ CompiledStaticPolicy)
CHOSEN_ARCHITECTURE           = SYNC_CFD_NATIVE_STATIC_BACKENDKIND (mirror FastCGI plane)
ARCHITECTURE_RATIONALE        =
  extends Cap033 route ownership SSOT; Tokio/Hyper=0; no Cap067 remingle;
  one-request one-handler; generation-published policy; fail-closed schema bump
```

### Rejection criteria (enforced)

No architecture admitted that requires: per-request control RPC; per-request cross-process IPC; Tokio in CFD; Hyper on CFD static path; global hot mutex/RwLock; WP/Laravel-specific static logic; filesystem sidecar as core native Static; request double-dispatch; transparent fallback after response commit; arbitrary Static miss→Proxy/FastCGI; security weaker than Cap004 product static path.

---

## 7. Ownership, precedence, miss, front-controller

```text
TARGET_REQUEST_OWNERSHIP_MODEL =
  request → Cap033 route selection → exactly one BackendKind → exactly one execution path
```

```text
TARGET_STATIC_PROXY_FASTCGI_ROUTE_PRECEDENCE =
  Cap033 host rank (exact > wildcard > hostless) then longest path prefix
  Example: /static/* Static, /api/* Proxy, /app/* Fastcgi coexist in one generation
  Overlaps: longer prefix wins; same prefix different hosts → host rank wins
  / and missing route → route_miss 404 (today)
```

```text
STATIC_MISS_CONTRACT =
  Static-owned route + missing/non-servable object → native Static 404 (or 403 on escape)
  NEVER Static miss → Proxy / FastCGI fallback after Static ownership
  Front-controller/rewrite must be modeled BEFORE dispatch if try_files-like behavior is desired
```

```text
STATIC_VS_FASTCGI_FRONT_CONTROLLER_BOUNDARY =
  /assets/* → BackendKind::Static (filesystem)
  /app/*    → BackendKind::Fastcgi + ADR-043 DI/FC (application policy)
  Do not implement try_files inside Static by calling FastCGI
```

---

## 8. Configuration / root / directory / MIME / methods

```text
STATIC_CONFIG_SSOT = generation-compiled CompiledStaticPolicy on the route (no per-request config parse)
```

| Field | Class |
| --- | --- |
| absolute docroot | MVP_REQUIRED |
| route path prefix (Cap033) | MVP_REQUIRED |
| index basenames (optional, basename-only) | MVP_REQUIRED (may be empty = no index) |
| MIME builtin extension table | MVP_REQUIRED |
| symlink policy = Cap004-aligned | MVP_REQUIRED |
| directory listing / autoindex | OUT_OF_SCOPE (off) |
| alias | OUT_OF_SCOPE (MVP) |
| Range | FOLLOWUP |
| ETag / IMS | FOLLOWUP (optional Last-Modified later) |
| precompressed | FOLLOWUP ([ADR-046](../adr/046-static-encoding-cache.md); Cap067-first MVP — was OUT_OF_SCOPE) |
| sendfile toggle | FOLLOWUP (Linux prefer-on default OK) |

```text
STATIC_MVP_CONFIG_FIELDS = docroot + Cap033 prefix + optional index list + symlink/file-type policy flags
STATIC_ROOT_ALIAS_DECISION = ROOT_ONLY_MVP (alias expands containment complexity; defer)
STATIC_DIRECTORY_CONTRACT =
  directory URI: try configured index basenames under root; if none → 404
  trailing slash: normalize per Cap033 path key; no autoindex
  Do not reuse FastCGI DirectoryIndex semantics on Static routes
STATIC_MIME_ARCHITECTURE = compile-time / process-lifetime builtin extension table (reuse product table idea)
STATIC_METHOD_CONTRACT = GET→body; HEAD→same headers zero body; other→405
STATIC_RANGE_SCOPE = FOLLOWUP_NOT_MVP (do not ship partial Range semantics)
STATIC_CONDITIONAL_REQUEST_SCOPE = FOLLOWUP_NOT_MVP
```

---

## 9. Transfer, sendfile, FD, identity

```text
STATIC_FILE_TRANSFER_MODEL =
  Linux: prefer sendfile(2) for non-trivial sizes; small files may write from bounded read
  Fallback: bounded pread → client write (fixed buffer; never load whole large file)
  mmap/splice: OUT_OF_SCOPE for MVP
```

```text
CFD_SENDFILE_FEASIBILITY = FEASIBLE_WITH_NEW_CONN_PHASE
WHY =
  Today queue_response only stages bytes into to_client_head/to_client (shard.rs).
  Cap067 pump_sendfile_nb needs OwnedFd + offset + EAGAIN park interests.
  Architecture admits a StaticSend phase on Conn (mirror Cap067 FSM), not buffer-only.
  Nonblocking client sockets already WouldBlock-aware.
STATIC_FALLBACK_IO_MODEL = bounded buffer pread/write; no global block; no unbounded Vec
```

```text
STATIC_FD_OWNERSHIP_MODEL =
  request-local OwnedFd for MVP; close on response complete / error / disconnect
  must not outlive request in MVP; generation drop does not leave leaked FDs
  optional later: shard-local FD cache keyed by (gen_id, root_id, relpath) + (dev,ino)
FILE_IDENTITY_ORACLE = open (openat2/openat) then fstat on FD; validate regular file + containment
OPENAT2_ROLE =
  Linux preferred RESOLVE_BENEATH (+ NO_MAGICLINKS as available);
  ENOSYS → portable fallback with O_NOFOLLOW + verified final path under root (Cap004 parity)
  Not sole architecture: Darwin/dev uses portable semantic core + PlatformStaticIo
```

```text
STATIC_SYMLINK_POLICY = in-docroot symlink OK iff final target remains under docroot; outside blocked
STATIC_SYMLINK_TOCTOU_INTERACTION = BOUNDED_RESIDUAL_PRESERVED; POTENTIAL_PHASE8_HARDENING_LEVER via FD containment
# Do NOT claim TOCTOU_CLOSED
```

```text
STATIC_PORTABILITY_ARCHITECTURE =
  CompiledStaticPolicy (portable) + PlatformStaticIo (Linux sendfile/openat2; Darwin read/write)
  macOS = LOCAL_ITERATION; Linux Netcup+Oracle for production evidence
```

---

## 10. Generation, reload, races, cache

```text
GENERATION_SCHEMA_CHANGE_REQUIRED = YES
GENERATION_SCHEMA_PROPOSED_CHANGE =
  CFDRT005 (name illustrative; exact magic chosen at impl)
  BackendKind::Static = 3
  CompiledStaticPolicy { root_path, index[], flags }
  fail-closed reject unknown kinds / bad roots
STATIC_GENERATION_VALIDATION =
  root absolute + normalized at publish; exists-at-publish recommended fail-closed
  index basename-only (reuse ADR-043 validators)
  no alias in MVP; reject traversal tokens in index; duplicate route ambiguity fail-closed
STATIC_RELOAD_MODEL =
  G1→G2 replaces CompiledStaticPolicy atomically with generation publish
  CONFIG_RELOAD != FILE_CONTENT_RELOAD (file mtime/content independent of gen)
STATIC_FILE_MUTATION_MODEL =
  serve opened FD snapshot semantics; replacement may yield short read / error after commit → close conn
STATIC_FILE_RACE_MODEL =
  security: containment on open FD; content consistency: best-effort POSIX; truncate mid-sendfile → error class after commit (no handler fallback)
STATIC_CACHE_DECISION = NO_STATIC_CACHE_INITIAL_MVP
STATIC_MEMORY_MODEL =
  per-route: CompiledStaticPolicy bytes; per-gen: table; per-request: path buf + optional send state; no unbounded per-file map
```

---

## 11. Concurrency, slow client, framing, errors, security

```text
STATIC_CONCURRENCY_MODEL =
  sync shard; open/stat/send may occupy shard until EAGAIN park or complete
  cold disk / NFS unsupported as production contract unless later admitted
STATIC_SLOW_CLIENT_MODEL =
  large file + slow client → Conn parked on EPOLLOUT; other conns on shard still progress
  one conn does not take a global lock; shard fairness = existing mio loop
STATIC_BACKPRESSURE_MODEL = EAGAIN park + bounded buffers; never read whole file into RAM
STATIC_RESPONSE_FRAMING_MODEL =
  status + Content-Length + Content-Type + Connection; optional Last-Modified later
  integrate with existing client_response / StaticSend header then body
STATIC_H1_KEEPALIVE_MODEL =
  preserve keepalive when full response committed cleanly (small/large/HEAD/404);
  partial write / disconnect → close; no half-poisoned reuse
STATIC_ERROR_MODEL =
  ENOENT→404; EACCES→403; ENOTDIR/ELOOP/escape→403/404 fail-closed;
  open/fstat fail before commit → error response;
  after commit → close connection; NEVER fall through to Proxy/FastCGI
```

```text
STATIC_PATH_NORMALIZATION_MODEL =
  reject .. / NUL / overlong / mixed escapes; decode once; Cap033 path key; join under root
CLIENT_PATH_CANNOT_ESCAPE_STATIC_ROOT = YES (openat2 RESOLVE_BENEATH or Cap004-equivalent)
STATIC_DYNAMIC_SOURCE_DISCLOSURE_POLICY =
  config: do not point Static docroot at PHP application trees;
  MVP MUST deny serving path segments ending in .php (case-insensitive) under Static routes;
  exclusive-docroot fail-closed at publish is an allowed additional control, not a substitute for .php deny
STATIC_DOTFILE_POLICY = DENY_BY_DEFAULT_MVP (leading-dot path segments)
STATIC_FILE_TYPE_POLICY = REGULAR_FILES_ONLY
STATIC_OBSERVABILITY_MODEL =
  handler=static; status; bytes; latency; error_class (no high-cardinality paths)
```

---

## 12. Hot-path / allocation / types

```text
EXPECTED_STATIC_HIT_COST_MODEL =
  Cap033 lookup + path normalize + openat2/open + fstat + MIME table + header format
  + sendfile/write loop; 0 global locks; 0 control RPC
NEW_GLOBAL_MUTEX_OPS_PER_REQUEST = 0
NEW_GLOBAL_RWLOCK_OPS_PER_REQUEST = 0
PER_REQUEST_CONTROL_PLANE_RPC = 0
CROSS_PROCESS_RPC = 0
TOKIO_IN_CFD = 0
HYPER_IN_CFD = 0
STATIC_ALLOCATION_MODEL =
  prefer stack/smallbuf path join; PathBuf acceptable MVP; avoid Arc/Box<dyn> per request
PROPOSED_STATIC_REQUEST_STATE = StaticSend { fd, offset, remaining, head_only, close }
PROPOSED_STATIC_TYPES =
  CompiledStaticPolicy, CompiledStaticRoot, StaticPathResolution,
  OpenedStaticFile, StaticResponsePlan, BackendKind::Static
```

```text
STATIC_ROUTE_COMPILATION_PIPELINE =
  config → validate → encode CFDRT005 → publish gen → RouteTable load → BackendTarget::Static
TARGET_STATIC_EXECUTION_PIPELINE =
  HTTP → lookup → Static policy → normalize → open/validate → headers → transfer → complete
```

---

## 13. Migration value + future MVP

```text
DOES_NATIVE_STATIC_CLOSE_MIGRATION_TOPOLOGY_GAP = YES
  # after implementation + Phase 7 requal; admission alone does not close evidence
```

```text
RECOMMENDED_IMPLEMENTATION_MVP =
  BackendKind::Static + CFDRT bump + root-only + GET/HEAD + MIME + CL
  + Cap004 open + REGULAR_FILES_ONLY + sendfile OR bounded fallback
  + 404/403 + Cap033 coexistence + observability
  EXCLUDE: autoindex, multi-range, FD cache, H2/H3, alias
  PRECOMPRESSED = FOLLOWUP (ADR-046 Cap067-first; CFD Static follows after product plane)
RECOMMENDED_NEXT_WIP = V044_CFD_NATIVE_STATIC_FILESYSTEM_HANDLER_IMPLEMENTATION
NEXT_WIP_AUTHORIZED = NO
P7_REQUALIFICATION_AFTER_STATIC_IMPLEMENTATION = REQUIRED
```

Future Netcup reality tests: **real filesystem files** — not proxy-to-Python, not mock FS, not hardcoded body bytes.

---

## 14. Integrity

```text
ZERO_FAKE = ABSOLUTE
NO_SMOKE = ABSOLUTE
ANTI_KOBAYASHI = PASS
  # no test-only /static special-case; no sidecar disguised as native; no fixture bytes as architecture
```

---

## 15. Specialist / auditor slots

Filled in terminal block from independent Task reviews (vertex / security / research / GRC / logic).

Nonblocking qualifications justifying **P7STATICADM-B** (not A):

1. Exact CFDRT005 on-wire layout bytes deferred to implementation WIP (schema **change required** is admitted; bit layout not frozen here).
2. `StaticSend` Conn-phase FSM detail is architecturally required but not line-specified (avoid false precision).
3. Symlink TOCTOU remains Phase-8 residual (by design).

No architecture P0/P1 MUST_FIX that blocks admission of `BackendKind::Static`
(ADR Accept A). CFDRT005 wire layout, StaticSend FSM detail, and Phase-8 TOCTOU
residual are **nonblocking qualifications** for CASE B (not architecture MUST_FIX).

Specialist / auditor results:

| Role | Verdict |
| --- | --- |
| Vertex | PASS (P7STATICADM-B) |
| Security | PASS (architecture docs; mutation still unauthorized) |
| Research | PASS |
| GRC | PASS |
| Logic | PASS_WITH_NONBLOCKING_P3; `ARCHITECTURE_ADMISSION_FALSIFIED=NO` |

---

## 16. STOP

```text
IMPLEMENTATION_AUTHORIZED = NO
NATIVE_STATIC_IMPLEMENTATION_FEASIBLE = YES (architecture)
NEXT_WIP_AUTHORIZED = NO
PHASE_7_REQUALIFICATION_AUTHORIZED = NO
PHASE_8_AUTHORIZED = NO
CANONICAL_MERGE = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION = NO
NOTION_RECONCILIATION_REQUIRED = YES
```

Owner review required before any `BackendKind::Static` code.
