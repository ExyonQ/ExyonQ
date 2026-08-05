# exyonq-mod-fastcgi

Plan 08 FastCGI / PHP-FPM transport module for ExyonQ.

**PR3-A:** bounded in-memory FastCGI v1 record parser (`record.rs`, `parser.rs`), inert transport traits (`transport.rs`), and inert `PhpFpmClient` skeleton (`client.rs`). Fuzz: `fuzz/fuzz_targets/fcgi_record.rs`. No wire I/O, no sockets, no core coupling.

**PR4-A:** in-memory mock FPM roundtrip (`encode.rs`, `mock.rs`, `PhpFpmClient<MockFpmTransport>::forward_once`). PARAMS + STDIN + empty STDIN terminator; mock responds with STDOUT + END_REQUEST. No `BEGIN_REQUEST`, no production sockets, no core hot-path wiring.

**PR5-A-min:** real unix/tcp wire transport module-only (`wire.rs`, `caps.rs`, `params.rs`). Scripted in-process peers in module tests (`tests/common/scripted_peer.rs`). Aggregate caps for PARAMS/STDIN/response. **No core diff.** **No live HTTP behavior change.**

Core returns **501 Not Implemented** for `Backend::Fastcgi` via the contract stub in `core/src/execute_backend.rs` — unchanged by PR5-A-min.

See [Plan 08 PR5 authorization packet](../../docs/architecture/plan08-pr5-authorization-packet.md) (§20 SIGNED PR5-A-min ONLY).

## PR5-A2-min — failure delegate adapter (2026-07-11)

§20 SIGNED PR5-A2-min ONLY — see [PR5-A2-min authorization packet](../../docs/architecture/plan08-pr5-a2-authorization-packet.md).

| Policy | PR5-A2-min |
|--------|------------|
| **`adapter.rs`** | Maps module transport/client errors → closed `FcgiDispatchOutcome` in `exyonq-module-api` |
| **Registration** | Test-harness only (`MockFcgiExecutor`); **no production binary registration** |
| **Live HTTP** | Success upstream → **501** (`SuccessNotAuthorized501`); failures → **502** / **504** when executor injected in tests |
| **Blocked** | Live **200**, live **503**, PR5-B, PR5-full, handler/wire_dispatch wiring |
| **Core coupling** | Core does **not** depend on this crate; adapter implements `FcgiBackendExecutor` only |

### PR5-B hard invariant (not authorized by PR5-A2-min)

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

### Reaffirmed invariants (unchanged by PR5-A-min)

- Live FastCGI remains **501** — no live **200**, **502**, **503**, or **504**
- **No `core/` diff** — no `execute_backend`, `wire_dispatch`, or `handler` wiring
- **No mandatory Docker/php-fpm** in `xtask ci`
- **PR5-A2** (core delegate + live failure codes) and **PR5-B** (live 200/503, smoke) are **not** authorized by implication — separate §20 required
