# ADR 045 — Safe-by-default product crates + Linux FFI capsule

```text
ADR = 045
TITLE = Safe-by-default product crates + Linux FFI capsule
STATUS = ACCEPTED
DATE = 2026-09-12
PRODUCT_MUTATION = YES (policy + encapsulation program)
```

## Context

ExyonQ is a modular reverse proxy in Rust intended to compete with NGINX and
OpenLiteSpeed on correctness, security, speed, and efficiency. Spreading
`unsafe` across `core/`, `exyonq-mod-static`, `exyonq-mod-proxy`, and CFD
weakens the Rust differential (memory-safe product logic). Kernel syscalls
(`epoll`, `sendfile`, `openat2`, `dup`, `send`/`write`) still require FFI.

## Decision

1. **North star (co-equal):** correct HTTP semantics, Safe-by-default product
   logic, competitive speed, low CPU/RAM — no silent trade-offs.
2. **Safe-by-default:** product crates (`core`, `module-api`, `mod-*`, `config/*`,
   `cli/*`, `runtime-plan`, CFD dataplane logic, WAF, …) move toward
   `#![forbid(unsafe_code)]`.
3. **Allowlisted FFI capsule:** `exyonq-linux-ffi` (Tokio-free, Hyper-free) owns
   Linux syscall wrappers behind **Safe** APIs. `exyonq-platform-linux` may keep
   epoll worker `unsafe` until fully migrated into the same capsule allowlist.
4. **CFD:** `TOKIO_IN_CFD = 0`, `HYPER_IN_CFD = 0` unchanged; CFD may depend on
   `exyonq-linux-ffi` only (not Tokio/`exyonq-core`).
5. **New `unsafe` outside allowlist** = `REJECT_ARCHITECTURE` until ADR revision.
6. **CI gate:** `scripts/gates/unsafe-allowlist-gate.sh` fails if `unsafe {`
   appears outside allowlisted paths.
7. **Honesty:** do not claim “zero unsafe in the binary including Tokio/std”.

## Allowlist (initial)

```text
crates/exyonq-linux-ffi/**
crates/exyonq-platform-linux/**   # temporary until epoll migrated into linux-ffi
```

## Consequences

- Hot-path encapsulation must not claim perf wins/losses without measurement.
- Duplicate sendfile/openat2 in CFD and mod-static converge on capsule APIs.
- Release checklist references this ADR for unsafe review.

## Invariants

```text
TOKIO_IN_CFD = 0
HYPER_IN_CFD = 0
UNSAFE_OUTSIDE_ALLOWLIST = FORBIDDEN
PRODUCT_SEMANTIC_SHORTCUTS = FORBIDDEN
SILENT_OK_WITHOUT_HTTP_RESPONSE = FORBIDDEN
```
