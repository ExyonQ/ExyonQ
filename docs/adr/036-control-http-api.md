# ADR 036 — HTTP Control API (Capability 016)

```text
ADR = 036
TITLE = HTTP Control API
STATUS = ACCEPTED
DATE = 2026-08-08
CAPABILITY_ID = 016
FEATURE_ID = control-http-api
OWNER_ACCEPTANCE = ACCEPT
IMPLEMENTATION_AUTHORIZED = YES
REQUIRED_CURRENT_PRODUCT_CAPABILITY = YES
```

## Context

Capability 016 was historically `DOC_ONLY` / Plan 80 `FUTURE_DESIGN`. The owner
admitted it as a **required current product capability** so Nexus Panel and other
clients can replace `StubBackend` with a real ExyonQ REST control plane.

```text
CAP016_REALITY_STATUS_BEFORE = DOC_ONLY_FUTURE_DESIGN
CAP016_OWNER_PRODUCT_ADMISSION = YES
CONTROL_API_OWNER = EXYONQ
NEXUS_ROLE = CLIENT_AND_ADAPTER
```

## Decisions

1. Canonical wire contract is OpenAPI **3.1.2** at `docs/control-api/openapi-v1.yaml`.
2. Protocol is HTTP REST only. Protobuf / gRPC / tonic are **not** in Cap016.
3. Transports: Unix domain socket `/run/exyonq/api.sock` (primary) and
   `127.0.0.1:9090` (secondary). Binding `0.0.0.0` is forbidden.
4. Sockets remain distinct: `api.sock` ≠ `control.sock` ≠ Nexus `agent.sock`.
5. Architecture: handwritten `ControlService` use cases; Hyper HTTP adapter;
   Cap013 `KernelControlPort::request_reload` for apply. No `exyonqctl` shell-out.
   No second reload engine.
6. Server framework for Cap016: **Hyper** (workspace). Axum is not added.
7. Rust client codegen: **openapi-to-rust** → reqwest + rustls. Generated code is
   transport only; product logic stays handwritten.
8. Auth v1: header `X-ExyonQ-Server-Token` with token family `exq_sk_v1_…`.
   Fail-closed. No query-string secrets.
9. Cap016 MVP endpoints only:
   - `GET /api/v1/health` (`getHealth`)
   - `GET /api/v1/stats` (`getStats`)
   - `GET /api/v1/config` (`getConfig`)
   - `POST /api/v1/config/vhosts` (`createVhost`) — **CREATE-only**; duplicate → 409
   - `DELETE /api/v1/config/vhosts/{domain}` (`deleteVhost`) — requires `If-Match`
10. Config `revision` (Control API document) ≠ Cap013 runtime `generation`.
11. Mutations: validate → stage → Cap013 prepare/commit. Success ⇒ requested state
    active; failure ⇒ last-good preserved.
12. MVP must not change the active listen / http3_listen / server-count set
    (`EXY-RELOAD-0005`). Create attaches `server_name` + route to the primary
    server; it does not add a new `[[server]]` listener.
13. Kill switch: omit Control API bind / `control_api.enabled=false` without
    affecting data plane or `control.sock`.
14. Closure requires dual-Linux REAL E2E with data-plane proof — not OpenAPI or
    codegen alone.

## Consequences

- New leaf crate `exyonq-control-api` (+ client packaging) outside `core/`.
- Cargo.lock may change for Cap016-scoped client/codegen deps only.
- Plan 80 full surface (capabilities registry, sites CRUD, RBAC, dry-run, …)
  remains deferred unless separately admitted.
