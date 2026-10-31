# ADR-050 — Custom error pages (NG-04 / v0.5 F1)

**Status:** Accepted  
**Date:** 2026-10-17  
**Depends on:** Cap035 rematch, ADR-049 (ordering vs `fallback_rewrite`)  
**Contract:** CONDITIONAL ACCEPT (`exyonq-architecture-contract` 2026-10-17)

## Context

Nginx `error_page 404 /404.html` and Apache `ErrorDocument 404 /404.html` have no ExyonQ IR. Migrators reject or mark gap. Roadmap NG-04 requires migrated configs that serve a custom body with the original error status.

## Decision

### IR — server-scoped map only

```toml
[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]
error_pages = { "404" = "/404.html", "500" = "/50x.html" }
```

| Field | Type | Notes |
| --- | --- | --- |
| `error_pages` | map string→string | Keys parse as `u16` in `400..=599`; values Cap035 absolute paths |

No top-level `[error_pages]` in v0.5 (avoids dual attachment). No `=200`, `@named`, or `http://` targets.

### Validation

1. Status key ∈ `400..=599`.
2. Value: absolute path `/…`; reject `@`, `http(s):`, `//`, `?`, `#`, empty.
3. Same path rules as `validate_rewrite_target`.

### Runtime (after NG-03)

```text
… → fallback_rewrite (if any)
  → if terminal 4xx/5xx (plain/generic body) and map hit and depth=0:
       rematch mapped path once (query preserved)
       serve body; **force original status**
       depth=1 (never re-apply error_pages)
  → else unchanged response
```

Rematch failure → original plain-text error (fail-closed).

### Migrator

| Form | Emit |
| --- | --- |
| `error_page 404 /404.html` | `error_pages["404"]="/404.html")` |
| `error_page 500 502 /50x.html` | one entry per code |
| `ErrorDocument 404 /404.html` | same |
| `@loc`, `=200`, URL redirect | reject / Unsupported |

## Consequences

- Custom 404/50x bodies without changing status semantics.
- Named locations and status rewrite remain residual — SoT `F1_RESIDUALS` (local lab path, not published) `F1-R11`…`F1-R14`.
