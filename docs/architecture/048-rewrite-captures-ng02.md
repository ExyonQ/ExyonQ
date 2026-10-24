# ADR-048 — Rewrite with captures and flags (NG-02 / v0.5 F1)

**Status:** Accepted  
**Date:** 2026-10-17  
**Depends on:** ADR-047 (NG-01 match)

## Context

Cap035 supports only a literal absolute-path `rewrite = "/target"` with a single rematch (`last` semantics). NGINX `rewrite ^/blog/(.*)$ /index.php?q=$1 last` and Apache `RewriteRule` with `$1` cannot be expressed. Roadmap NG-02 requires `$1..$9`, flags `last`/`break`/`redirect`/`permanent`, and minimal `RewriteCond` equivalents.

## Decision

### IR

`route.rewrite` is an untagged enum:

| Form | TOML | Semantics |
|------|------|-----------|
| Literal | `rewrite = "/app/index.html"` | Cap035 unchanged: absolute path, rematch once |
| Rule | `rewrite = { pattern, replacement, flag?, when? }` | Regex against request path; expand `$1..$9` / `$$` |

`flag` default = `last`.

| Flag | Behavior |
|------|----------|
| `last` | Internal URI → rematch routes once (Cap035) |
| `break` | Internal URI → stay on current route (no rematch) |
| `redirect` | HTTP 302 to expanded location |
| `permanent` | HTTP 301 to expanded location |

`when` is an AND list of conditions (optional). Each entry may set one of:

- `path` — regex against request path  
- `host` — exact host (Cap033 normalize)  
- `query` — regex against raw query (no `?`)  
- `header` — header name must be present and non-empty  
- `header_value` — if set with `header`, exact value match (case-sensitive for value)

Empty `when` → always apply when pattern matches.

### Compilation

Pattern and `when.path` / `when.query` compile at plan build (with match.regex). Never on the hot path. Pattern length ≤ 256.

### Replacement

Only `$1`…`$9` and `$$`. No named captures, no nginx variables (`$uri`, `$host`). After expansion:

- `last`/`break`: must be a Cap035-valid absolute path (no `?`/`#`/authority)  
- `redirect`/`permanent`: Cap036 location rules (absolute path or absolute URL; no CTL)

### Migrator

`rewrite` with captures + known flags maps to `Rewrite` rule form. Literal-safe redirect mapping unchanged.

## Consequences

- Existing Cap035 configs unchanged.  
- Break enables front-controller patterns without rematch loops.  
- Query injection via `$1` into path is fail-closed for last/break (no `?` in path). Apache OR/`!`/QSA/PT/END and broad `.htaccess`: SoT [`F1_RESIDUALS`](../../ROADMAP%20EXYONQ/F1_RESIDUALS.md) `F1-R03`…`F1-R07`.
