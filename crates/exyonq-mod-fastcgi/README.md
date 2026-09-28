# exyonq-mod-fastcgi

Plan 08 FastCGI / PHP-FPM transport module for ExyonQ.

**PR3-A:** bounded in-memory FastCGI v1 record parser (`record.rs`, `parser.rs`), inert transport traits (`transport.rs`), and inert `PhpFpmClient` skeleton (`client.rs`). Fuzz: `fuzz/fuzz_targets/fcgi_record.rs`. No wire I/O, no sockets, no core coupling.

**PR4-A:** in-memory scripted FPM peer (`encode.rs`, `scripted.rs`, `PhpFpmClient<ScriptedFpmTransport>::forward_once`) for protocol unit tests. Production uses `FcgiModuleExecutor` + unix/tcp wire. PARAMS + STDIN; scripted peer responds with STDOUT + END_REQUEST.

**PR5-A-min:** real unix/tcp wire transport module-only (`wire.rs`, `caps.rs`, `params.rs`). Scripted in-process peers in module tests (`tests/common/scripted_peer.rs`). Aggregate caps for PARAMS/STDIN/response. **No core diff.** **No live HTTP behavior change.**

Core fail-closes with HTTP **501** only when **no** FastCGI service is registered.
With CLI `FcgiModuleExecutor` registration, live FastCGI returns real CGI status
(200 / 502 / 503 / 504) over unix/tcp to PHP-FPM.

See [Plan 08 PR5 authorization packet](../../docs/architecture/plan08-pr5-authorization-packet.md)
for historical phase packets (PR5-A-min / PR5-A2-min). Current product path supersedes
those “live HTTP still 501” rows.

## Current product path (post PR5-B / CLI registration)

| Policy | Current |
|--------|---------|
| **Registration** | CLI registers `FcgiModuleExecutor::production_pools` |
| **Live HTTP** | Success → real CGI status (incl. **200**); wire failures → **502** / **503** / **504** |
| **Scripted peers** | Unit/composition only (`ScriptedFcgiExecutor` / `ScriptedFpmTransport`) — not CLI serve |
| **Unregistered** | Core → **501** fail-closed |

## Historical — PR5-A2-min failure delegate adapter (2026-07-11)

§20 SIGNED PR5-A2-min ONLY at the time — see [PR5-A2-min authorization packet](../../docs/architecture/plan08-pr5-a2-authorization-packet.md).
**HISTORICAL / SUPERSEDED** by CLI live registration above.

| Policy | PR5-A2-min (historical) |
|--------|-------------------------|
| **`adapter.rs`** | Maps module transport/client errors → closed `FcgiDispatchOutcome` in `exyonq-module-api` |
| **Registration** | Unit/composition only at that phase |
| **Live HTTP (then)** | Success upstream → **501** (`SuccessNotAuthorized501`) — **no longer current** |
| **Core coupling** | Core does **not** depend on this crate; adapter implements `FcgiBackendExecutor` only |

### PR5-B hard invariant (async offload)

PR5-B **must not** reuse `block_in_place` + blocking wire dispatch on Tokio worker threads. PR5-B requires a reviewed async offload boundary (`spawn_blocking` or dedicated pool + timeout budget) before any live executor registration.

## PR5-A-min — §20 merge deferrals (2026-07-09)

Accepted with **CONDITIONAL ACCEPT** from security-compiler and local-debugger; **ACCEPT** from architecture-contract. Docs-only record — no runtime change.

### Deferred (Medium / Low)

| Item | Disposition |
|------|-------------|
| **`fcgi_request_timeout`** (300s, packet §7) | Deferred to **PR5-A2** / async Tokio path |
| **`UnixStream::connect` without timeout** | Accepted for PR5-A-min module tests only; resolve in PR5-A2 if used on a real hot path |
| **`wire_caps.rs` STDIN cap test** | Allocates ~32 MiB + 1 byte to validate cap; accepted while CI passes — monitor memory on Linux CI |
| **Unix socket temp dir cleanup** | Low/defer — `$TMPDIR/exyonq-fcgi-peer-{pid}/` may accumulate in repeated local runs |
| **TCP peer 20 ms sleep** | Low/defer — anti-race before connect; no change required while CI is stable |
| **`PeerMode::SlowRead`** | Future injection only — not exercised in CI to avoid long read-timeout hangs |

### Reaffirmed invariants (unchanged by PR5-A-min phase)

- At PR5-A-min freeze: live FastCGI remained **501** (no live **200**/**502**/**503**/**504** yet)
- **No `core/` diff** in that phase — no `execute_backend` / `wire_dispatch` / `handler` wiring then
- **No mandatory Docker/php-fpm** in `xtask ci`
- Later phases (CLI `FcgiModuleExecutor` + PR5-B) **superseded** the “live remains 501” freeze — see **Current product path** above
