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
| --- | --- | --- |
| Literal | `rewrite = "/app/index.html"` | Cap035 unchanged: absolute path, rematch once |
| Rule | `rewrite = { pattern, replacement, flag?, when? }` | Regex against request path; expand `$1..$9` / `$$` |

`flag` default = `last`.

| Flag | Behavior |
| --- | --- |
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

`redirect`/`permanent` append the original query string when the expanded
replacement has no `?` of its own, matching nginx/Apache behavior. Internal
`last`/`break` targets remain absolute paths and never carry a query.

### Matching form

Patterns (and `when.path`) match nginx `$uri`: the request path decoded once and
normalized, never the request-target as received. The kernel canonicalizes the
target to URI form (`%20` stays escaped, H-01), so the engine decodes one pass to
match, runs the rule, and re-encodes the expanded replacement before validating
or forwarding it. A decoded capture therefore never reaches the upstream as a
literal space, and a `%` inside a capture is escaped as `%25` instead of being
read back as an escape. When the decoded bytes are not UTF-8 (for example `%FF`)
the text rule first gets the URI form, exactly as before, so a rule written
against the escaped spelling keeps matching and its captures still pass through
unescaped. If that does not match, the same pattern runs byte-wise
(`regex::bytes`, Unicode disabled) over the decoded bytes, which is what nginx
does: `^/i.ndex$` matches `/i\xFFndex` and `$1` is re-encoded to `%FF` on the way
out. `$uri` in that branch is the decoded path, and the captures are escaped once.
`when.path` obeys the same rule and in the same order: the URI form first, then
the decoded bytes through its own byte-wise twin, so a condition and the pattern
it guards never disagree about what the path is. `when.query` and the header
conditions are unaffected — the query string is not percent-decoded and header
values arrive as text.

### Compilation

Pattern and `when.path` / `when.query` compile at plan build (with match.regex). Never on the hot path. Pattern length ≤ 256.

### Replacement

`$1`…`$9`, `$$`, and the narrow ADR-062 allowlist `$uri` / `$host`. No named
captures or other nginx variables. After expansion:

- `last`/`break`: must be a Cap035-valid absolute path (no `?`/`#`/authority)  
- `redirect`/`permanent`: Cap036 location rules (absolute path or absolute URL; no CTL)

### Migrator

`rewrite` with captures + known flags maps to `Rewrite` rule form. Literal-safe redirect mapping unchanged.

## Consequences

- Existing Cap035 configs unchanged.  
- Break enables front-controller patterns without rematch loops.  
- Query injection via `$1` into path is fail-closed for last/break (no `?` in path). Apache OR/`!`/QSA/PT/END and broad `.htaccess`: SoT `F1_RESIDUALS` (local lab path, not published) `F1-R03`…`F1-R07`.
