# Phase 1 — final acceptance packet

```text
DOCUMENT = docs/architecture/phase1-final-acceptance.md
PROGRAM = phase1-strategic-closure.plan.md (C0–C6)
DATE_UTC = 2026-10-12
LAB_HEAD = e7a649f3bd6326ddc3be62669c77f7e4a718fec2
PUBLIC_RELEASE_LINE = v0.4.11 (exyonq-github; not this lab tip)
WASMTIME_LAB = 49.0.2 (bumped from 46.0.2; owner-authorized)
PHASE1_STATUS = CONDITIONAL_CLOSE — E2E dual-Linux PASS; cargo-audit clean; p1.1 soak PASS; p1.3a soak FAIL (PROXY_DRAIN); C4 protectors not re-run; Veyra not ACCEPT (anti-theater warnings / flaky workspace cargo-test)
PROJECT_PHASE_1_STATUS = NOT_CLOSED
R6 = NOT_STARTED
COMMIT = NO
PUSH = NO
DEPENDENCY_UPDATE = AUTHORIZED (this session: wasmtime/wat/h2/rustls scoped)
```

## Scope claimed (single-node)

```text
static · reverse proxy · TLS · HTTP/1.1 · HTTP/2
FastCGI/PHP-FPM · WordPress path · microcache v0
reload/drain/shutdown · observability · TOML IR + validate
NGINX Tier 1/2 import · Apache equivalent importer (0.4.11)
7 product profiles (static|proxy|php|wordpress|websocket|sse|tls-h2)
```

Out of scope (unchanged): cluster/mesh, ExyApp, ACME complete, `.exy` parser, HTTP/3 production claims, Redis/distributed cache.

## C0–C6 rollup

| Gate | Verdict | Evidence |
| --- | --- | --- |
| **C0** Canonical state | **CLOSED** (historical) | plan + tracker |
| **C1** Release gate (v0.3.3 era) | **SUPERSEDED** by public cuts through **v0.4.11** | GitHub release + GHCR |
| **C2** FastCGI production-grade | **ACCEPT (host)** | FCGI lib **64/64**; policy [`fcgi-saturation-policy-v1.md`](fcgi-saturation-policy-v1.md); dual-Linux fcgi E2E **PASS** |
| **C3** Config / migration UX | **ACCEPT (host)** | `check` / `dump-ir` / 7 profiles; merge+cli tests green |
| **C4** Performance convergence | **OPEN** | No new official ReWrk protector matrix this session |
| **C5** Security / soak | **PARTIAL ACCEPT** | `cargo audit` **clean** (Wasmtime **49.0.2**, rustls **0.23.45**, h2 **0.4.20**; yanked `chacha20` warning only). Dual-Linux E2E **PASS**. **p1.1 soak PASS** dual-arch 180s. **p1.3a soak FAIL** (PROXY_DRAIN new=000 both arches; also cert-reload / H2 gates). Veyra quick: tooling OK (`fmt`/`check`/`clippy`/`audit`) but decision **REJECTED** (anti-theater VT* warnings and/or flaky workspace `cargo test`) |
| **C6** This packet | **CONDITIONAL** | See blockers |

## Dependency bump (authorized)

| Package | From | To |
| --- | --- | --- |
| wasmtime (workspace pin) | `=46.0.2` | `=49.0.2` |
| wat (exyonq-wasm-host) | `1` / 1.254.0 | `=1.260.0` |
| h2 | 0.4.15 | 0.4.20 |
| rustls | 0.23.40 | 0.23.45 |

Host: `cargo test -p exyonq-wasm-host --lib` **7/7**. `cargo audit` exit **0** (allowed yank warning only).

Collateral product fix (not dep): Cap055 `publish_wire_module_state()` on COMMIT in `core/src/server/state.rs` — integration `ratelimit_returns_429_when_exceeded` now **PASS**.

## Dual-Linux production E2E

```text
EVIDENCE = .exyonq-local/tmp/phase1-e2e-20261012T122123Z/
```

| Arch | static | proxy | fcgi | http2 | tls |
| --- | --- | --- | --- | --- | --- |
| amd64 | PASS | PASS | PASS | PASS | PASS |
| arm64 | PASS | PASS | PASS | PASS | PASS |

## Soaks (optional, executed)

| Soak | Run ID | Duration | amd64 | arm64 |
| --- | --- | --- | --- | --- |
| p1.1 correctness/stability | `p1-1-soak-wasm49-20261012T131801Z` | 180s | **PASS** | **PASS** |
| p1.3a proxy/TLS/H2 | `p13a-soak-wasm49-20261012T133554Z` | 180s + 15s warmup | **FAIL** (PROXY_DRAIN; 58 PASS / 11 FAIL gates) | **FAIL** (same PROXY_DRAIN pattern) |
| p1.2 wordpress / p1.3b http3 | — | — | **not run** this session | — |

Reports: `docs/operations/p1.1-soak-{amd64,arm64}-report.md`, `docs/operations/p1.3a-proxy-tls-h2-soak-{amd64,arm64}-report.md`.

## Veyra

```text
KEY_WIN = cargo-audit OK after Wasmtime 49.0.2
LATEST_QUICK = .veyra/evidence/20261012T132947Z (audit OK; REJECTED on anti-theater findings policy)
WITH_CARGO_TEST = intermittent REJECTED on plan12 ttl_race under full-workspace parallel (isolated re-run PASS)
```

## Blockers before `PROJECT_PHASE_1_STATUS = CLOSED`

1. **p1.3a PROXY_DRAIN** — drain path returns `new=000` instead of expected 503 (both arches)
2. **C4** protector classification against current binary
3. **Veyra ACCEPT** — either tune fail-on for VT warnings or remediate findings; stabilize workspace parallel tests
4. Optional: p1.2 / p1.3b soaks

## Phase 2 / R6

```text
R6_STATUS = NOT_STARTED
```

## Sign-off

| Role | Status |
| --- | --- |
| Dual-Linux E2E gates | **ACCEPT** |
| cargo-audit (lab lock) | **ACCEPT** |
| p1.1 soak | **ACCEPT** |
| p1.3a soak | **REJECT** |
| Veyra full ACCEPT | **NO** |
| Phase 1 CLOSED | **NO** — CONDITIONAL |
