# ADR 044 — CFD native static filesystem handler architecture

```text
ADR = 044
TITLE = CFD native static filesystem handler architecture
STATUS = ACCEPTED (ARCHITECTURE) — IMPLEMENTATION WIP V044_CFD_NATIVE_STATIC_FILESYSTEM_HANDLER_IMPLEMENTATION (P7STATICADM-B child; not P7 requal)
DATE = 2026-09-01
WIP = V044_CFD_NATIVE_STATIC_FILESYSTEM_HANDLER_ARCHITECTURE_ADMISSION (sealed) → child implementation WIP authorized by owner 2026-09-01
PARENT = P7-E (docs/governance/v044-phase7-static-proxy-coexistence.md)
PRODUCT_MUTATION = YES (implementation WIP; see docs/governance/v044-cfd-native-static-filesystem-handler-implementation.md)
IMPLEMENTATION = IN_PROGRESS (native Static MVP in exyonq-cfd-*; Netcup bounded qualification — not Phase 7 requal)
NOTE = BackendKind::Static + CFDRT005 + CompiledStaticPolicy present in ambient lab tree at implementation WIP; ADR decision unchanged (Accept A).
```

Governance binder:
[`docs/governance/v044-cfd-native-static-filesystem-handler-architecture-admission.md`](../governance/v044-cfd-native-static-filesystem-handler-architecture-admission.md)

## Context

P7-E (Netcup) proved same-generation **Proxy + FastCGI** isolation and falsified reachability of native three-handler coexistence: CFD `BackendKind` is `Proxy | Fastcgi | Reject` only (`crates/exyonq-cfd-gen/src/route_table.rs`). Cap067 is product-plane epoll static (`exyonq-mod-static`), not a CFD handler. P6G “static” via Proxy→Python fileserver is **not** native Static proof.

```text
NATIVE_STATIC = CFD opens and serves the filesystem object itself
```

## Options considered

| ID | Option | Latency | CPU | Mem | Complexity | Security | Reuse | Ownership | Isolation | Migration | Portability | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| I | NO_NEW_PATTERN | — | — | — | low | n/a | n/a | broken | n/a | no | n/a | **REJECT** — no CFD handler opens files |
| A | `BackendKind::Static` | low | low | low | med | Cap004-class | Cap067 patterns | exclusive | high | yes | policy+PlatformIo | **ACCEPT** |
| B | `HandlerKind`/`RouteAction` rename | same | same | same | churn | same | same | same | same | yes | same | REJECT rename-only |
| C | Static policy outside BackendKind | same | same | same | higher | OK | OK | ambiguous SSOT | OK | yes | OK | REJECT |
| D | Generic response-source executor | unknown | unknown | unknown | high | OK | over-abstract | diffuse | OK | yes | OK | REJECT MVP |
| E | Proxy local static sidecar | +hop | + | + | low false | weaker truth | Proxy | false Static | process | fake yes | OK | **REJECT** (not native) |
| F | Per-request product Cap067 handoff | high | high | — | med | mixed planes | Cap067 | dual | weak | partial | — | **REJECT** |
| G | Main-process static IPC | high | — | — | high | RPC risk | Cap067 | dual | weak | partial | — | **REJECT** |
| H | No native CFD static forever | — | — | — | low | — | — | absent | — | **no** | — | **REJECT** as final arch |

## Decision

**Accept A — sync CFD-native `BackendKind::Static`** (architecture only):

1. **Plane:** Execute in `exyonq-cfd-dataplane` mio shard (Tokio=0, Hyper=0).
2. **IR:** Future CFDRT bump adds `BackendKind::Static` + `CompiledStaticPolicy` (root-only MVP; no alias).
3. **Ownership:** Cap033 unchanged; one route → one handler; Static miss → Static 404 (no Proxy/FastCGI fall-through).
4. **vs FastCGI FC:** ADR-043 DI/FC stays FastCGI-route-only; Static is separate ownership.
5. **Cap067:** Product plane preserved. Reuse **patterns** (openat2 RESOLVE_BENEATH, fstat identity, sendfile/EAGAIN) via extract-or-reimplement — **do not** link `exyonq-mod-static` / `static_wire` into CFD.
6. **IO:** Prefer Linux sendfile via new `StaticSend` Conn phase (current `queue_response` is buffer-only); bounded pread/write fallback; no whole-file buffer; no FD cache in MVP.
7. **Security:** Cap004-class containment; REGULAR_FILES_ONLY; deny dotfiles by default; deny/escape fail-closed; PHP disclosure = config + MVP `.php` deny under Static; symlink TOCTOU = **BOUNDED_RESIDUAL** (Phase 8 review) — not closed here.
8. **Portability:** `CompiledStaticPolicy` + `PlatformStaticIo`.

## Consequences

- Architecture admitted; **no** product code changes from this ADR alone.
- Implementation requires separate owner WIP + pre-mutation reviews (avoid ADR-038 POSTHOC gap).
- Phase 7 three-handler VERIFIED remains blocked until Static MVP + real FS Netcup evidence.
- Phase 8 / async FastCGI / competitive Static benches remain unauthorized.

## Invariants

```text
TOKIO_IN_CFD = 0
HYPER_IN_CFD = 0
CONTROL_PLANE_RPC_PER_REQUEST = 0
CROSS_PROCESS_RPC_PER_REQUEST = 0
NEW_GLOBAL_HOT_LOCK = FORBIDDEN
CAP067_PRODUCT_PLANE = PRESERVED
PROXY_AS_NATIVE_STATIC = FORBIDDEN
STATIC_MISS_FALLTHROUGH = FORBIDDEN
TOCTOU_CLOSED_CLAIM = FORBIDDEN (residual preserved)
IMPLEMENTATION_FROM_THIS_ADR_ALONE = FORBIDDEN
```

**Later event (does not change ADR STATUS=ACCEPTED):** Phase 8 (2026-09-01) found Linux `openat2` FIFO partner-wait hang (`P8-STATIC-FIFO-001`, CASE P8-C). Repair requires a separate owner WIP. TOCTOU remains not closed. Binder: `docs/governance/v044-phase8-completeness-and-adversarial-security.md`.

**Amendment (2026-09-22):** Static **precompressed / encoding cache** is no longer permanently OUT_OF_SCOPE. [ADR-046](046-static-encoding-cache.md) admits Cap067-first static gzip cache; CFD Static remains FOLLOWUP (no Cap067↔CFD handoff).
