# ADR-062 — Transition pack: route `response_headers` + rewrite `$uri`/`$host`

**Status:** Accepted (orchestrator + architecture-contract **CONDITIONAL ACCEPT**)
**Date:** 2026-10-18
**Closes (partial):** `F1-R19` (primary), `F1-R07`, narrow `F1-R15` (templates only)
**Amends:** ADR-048 (rewrite vars), ADR-051 (response templates subset), ADR-061 (non-goals)
**Does not claim:** NG-17 complete, F5 complete, 100% nginx/apache parity, F1 DoD reopen

## Context

Migration reports leave `add_header` / `Header set` as manual ⚠️ because F1 deferred route-scoped outbound headers (`F1-R19`). Rewrites that use `$uri` / `$host` fail closed (`F1-R07`). Operators need a **bounded unlock** before F5/F8 without cloning nginx inheritance or opening `$http_*`.

## Decision

### 1. IR (canonical)

On `[[route]]`, list-only:

```toml
[[route.response_headers]]
name = "X-Frame-Options"
value = "SAMEORIGIN"
always = false   # default: apply on 2xx/3xx only

[[route.response_headers]]
name = "X-Content-Type-Options"
value = "nosniff"
always = true    # apply on all statuses for this response path
```

- At most one entry per header **name** per route (case-insensitive).
- **No** table sugar in normative config (cannot express `always`).
- **No** `[[server.response_headers]]`.

### 2. Validation denylist (case-insensitive)

Reject: `Connection`, `Transfer-Encoding`, `Keep-Alive`, `Upgrade`, `TE`, `Trailers`, `Proxy-Authenticate`, `Proxy-Authorization`, any `Proxy-*`, **`Cache-Control`** (use `cache_headers`, ADR-061), **`Set-Cookie`**.

### 3. Response template allowlist (narrow R15)

Literals + `$host` + `$uri` only (`$uri` = path only, no query).
**Not** `$1`–`$9`, `$remote_addr`, `$scheme`, `$request_uri`, `$map`, `$http_*`, `$cookie_*`.

### 4. Runtime

- Apply after route match on **client-bound** responses for that route (static, proxy, FastCGI, generated errors still attributed to the matched route).
- After `cache_headers` where both apply.
- **Non-overwrite:** if response already has the header name, skip.
- `always=false` → status 200–399; `always=true` → any status.
- Custom `error_page` rematch uses the **rematched** route’s headers.

### 5. Rewrite (R07)

`expand_replacement` + IR validate accept `$uri` (request path) and `$host` (normalized: lowercase, strip trailing `.`, port stripped if present in Host). Existing `$1`–`$9` / `$$` unchanged.

### 6. Migrators

| Source | Mapping |
|--------|---------|
| nginx `add_header Name Value;` | `always=false` |
| nginx `add_header Name Value always;` | `always=true` |
| Apache `Header set Name Value` | `always=true` |
| Server-scoped headers | Attach to that server’s catch-all / DocumentRoot route |
| `Header append` / `edit` / `unset`, disallowed vars, hop-by-hop names | Fail-closed / report Unsupported |

### 7. Non-goals

Nginx level inheritance / duplicate merge simulation; `if`; open `$http_*`; `.htaccess` Header execution (`F1-R05`); `real_ip` (`F1-R18`); nginx `expires`; full NG-17; named `@` locations.

## Consequences

- Canonical ejemplos can migrate security headers without invalid TOML (fixes F1-W04 class of bug).
- F5 / NG-17 remainder (nginx `expires`, richer vars, inheritance) stays scheduled.
- `F1_RESIDUALS.md` marks R19/R07 resolved (date); R15 remains partially open for other surfaces.
