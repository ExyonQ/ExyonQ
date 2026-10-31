# ADR-051 — Variables mínimas + `match.header` + `map` (NG-05 / v0.5 F1)

**Status:** Accepted  
**Date:** 2026-10-18  
**Depends on:** ADR-047 (match), Cap033 host AND  
**Contract:** CONDITIONAL ACCEPT (`exyonq-architecture-contract` 2026-10-17)

## Context

Nginx `proxy_set_header X-Real-IP $remote_addr`, simple `map`, and header-gated routes have no ExyonQ IR. Migrators reject `proxy_set_header` / `map`; `if` must stay fail-closed by design.

## Decision

### Variable allowlist (exactly four built-ins)

| Var | Meaning |
| --- | --- |
| `$host` | Normalized request host (port stripped) |
| `$remote_addr` | Peer socket IP only (no `real_ip` / XFF trust) |
| `$request_uri` | Path + optional `?query` |
| `$scheme` | `https` if TLS listener, else `http` |
| `$<map.name>` | Exact map lookup (or map `default`) |

Unknown `$…` → config load error. No `${…}`, no `$http_*`, no nested expand.

### `match.header` — AND filter (not XOR)

```toml
match = { path = "/admin", header = "X-Admin", header_value = "1" }
```

- Still exactly one of `path` | `suffix` | `regex` (ADR-047).
- Optional `host` AND optional `header` (+ optional exact `header_value`).
- Header name match is case-insensitive; value is exact / case-sensitive.
- `header` without value → present and non-empty.
- Max one header constraint per route. Ranking unchanged (filter only).

### `proxy_set_headers` on upstream routes

```toml
proxy_set_headers = [
  { name = "X-Real-IP", value = "$remote_addr" },
  { name = "Host", value = "$host" },
]
```

Replace named headers on the **outbound upstream** request after rewrite/rematch. Only routes with `upstream`. Hop-by-hop denylist except explicit `Connection` when driven by a map.

### Top-level `[[map]]`

```toml
[[map]]
name = "connection_upgrade"
input = "header"
header = "Upgrade"
default = "close"
values = { "websocket" = "upgrade" }
```

Exact keys only; ≤64 entries; ≤32 maps. Inputs: `header` | `host` | `remote_addr` | `request_uri` | `scheme`.

### `if`

Migrator **Error** with diagnostic. No IR.

### Ordering

Match (path XOR + host AND + header AND) → rewrite/rematch → fallback/error_pages → if final action is upstream, expand `proxy_set_headers` for that route.

## Residuals (fail-closed)

SoT: `ROADMAP EXYONQ/F1_RESIDUALS.md` (local lab path, not published) — `F1-R15`…`F1-R21` (`if`, `set`, `geo`, regex `map`, `$http_*`, `real_ip`, route-scoped `add_header`, multi-header OR).

## Consequences

- `proxy_set_header X-Real-IP $remote_addr` migrates and is observable on upstream.
- Simple `map` → `$name` in header values.
- No expression language / `if` compatibility.
