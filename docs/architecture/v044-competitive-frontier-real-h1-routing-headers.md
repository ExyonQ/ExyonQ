# V044 Competitive Frontier — Phase 2: Real H1 GET + Routing + Headers

**WIP:** `V044_COMPETITIVE_FRONTIER_REAL_H1_ROUTING_HEADERS`  
**Phase:** 2 — STOP; Phase 3 (WAF) not authorized  
**Parent:** CFD-A foundations (`SEPARATE_PROCESS_HYBRID`)

## Owner-frozen architecture (inherited)

| Decision | Value |
| --- | --- |
| Process model | `HYBRID` — Tokio control + separate OS process dataplane |
| Dataplane runtime | std threads + mio (no Tokio) |
| Competitive H1 accept | dataplane only |
| Hyper fallback | Forbidden |
| Cap067 | Unchanged |
| WAF / request bodies | Out of scope (Phase 2) |

## Phase-2 product surface

Real HTTP/1.1 GET only:

1. Bounded request-line + header parse  
2. Cap033 compiled route projection (`RouteTable` / `CFDRT002`)  
3. Hop-by-hop request/response filtering (Connection tokens)  
4. Upstream Host = peer authority (`UPSTREAM_HOST_POLICY`)  
5. Client Host = Cap033 routing key only (`CLIENT_HOST_POLICY`)  
6. Shard-local upstream keepalive pool + poison discard  
7. Shard-local timers  
8. Client keepalive with per-request generation refresh  

## Host policies (product-aligned)

```text
CLIENT_HOST_POLICY = ROUTING_KEY_ONLY
UPSTREAM_HOST_POLICY = GET_REPLACE_WITH_PEER_AUTHORITY_HOST
ROUTING_SSOT = CONTROL_PRODUCT_PLANE
DATAPLANE_ROUTING = COMPILED_PROJECTION_ONLY
```

## Generation payload

Outer blob unchanged (`EXYQCFD1`). Payload is `CFDRT002` route table (or empty table).

Control projects via:

- `EXYONQ_CFD_ROUTES` text file (`host|path|connect|authority`), or  
- programmatic `CfdChild::publish_routes` / `table_from_entries`

No hardcoded `/api` routes. No second routing SSOT.

## Feature gate (default OFF)

```text
EXYONQ_COMPETITIVE_H1_DATAPLANE=1
EXYONQ_CFD_LISTEN=IP:PORT
EXYONQ_CFD_GEN_DIR=/path
EXYONQ_CFD_ROUTES=/path/to/routes.txt   # optional
EXYONQ_CFD_SHARDS=N                     # optional
EXYONQ_DATAPLANE_BIN=/path              # optional
```

## Explicit non-scope

WAF · request bodies · chunked response productization · H2/H3 · modules · FastCGI · Cap067 · full observability · Hyper fallback · Tokio in dataplane.

## Phase-3 gate

WAF generation/reload — **requires new owner authorization**. Do not autostart.
