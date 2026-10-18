# ADR-049 — `try_files` / `fallback_rewrite` (NG-03 / v0.5 F1)

**Status:** Accepted
**Date:** 2026-10-17
**Depends on:** ADR-047 (NG-01 match), ADR-048 (NG-02 rewrite), Cap035 rematch
**Contract:** CONDITIONAL ACCEPT (`exyonq-architecture-contract` 2026-10-17)

## Context

Nginx `try_files $uri /index.html` (SPA) and `try_files $uri $uri/ /index.php?$args` (WordPress) cannot be expressed in ExyonQ IR. Compat today classifies them Partial/Unsupported. Roadmap NG-03 requires a declarative equivalent with real E2E, not htaccess-only workarounds for nginx imports.

## Decision

### No full `try_files = [...]` array

Admit a fixed probe set + one rematch target (not a nginx expression list).

### IR fields on `RouteConfig`

| Field | Type | Default when `fallback_rewrite` set |
|-------|------|-------------------------------------|
| `fallback_rewrite` | `Option<String>` | — (Cap035-valid absolute path; no `?`/`#`) |
| `try_uri` | `Option<bool>` | `true` |
| `try_uri_dir` | `Option<bool>` | `false` |

### Validation

1. `fallback_rewrite` requires `root`.
2. XOR with unconditional `rewrite`, `redirect`, `upstream`, `fastcgi`.
3. XOR with `htaccess = overlay` (ADR-043 coexistence: nginx migrator emits NG-03; Apache WP keeps overlay without NG-03).
4. Target passes Cap035 `validate_rewrite_target`.
5. At least one of effective `try_uri` / `try_uri_dir` is true.
6. `try_uri` / `try_uri_dir` without `fallback_rewrite` → error.

### Runtime

On a matched route with `root` + `fallback_rewrite` (rematch depth 0):

1. Attempt normal static serve under `root` (covers `$uri` and directory index when the static module already does).
2. If response is **404** → set URI path to `fallback_rewrite`, **preserve original query**, rematch routes **once** (Cap035 `last`).
3. Rematched route executes normally; this mechanism does **not** re-apply `fallback_rewrite` (depth ≥ 1).

**WordPress `?$args`:** documented equivalent of preserving the request query. The IR target is `/index.php`, never `/index.php?$args`.

**SPA:** `try_uri=true`, `try_uri_dir=false`, `fallback_rewrite="/index.html"`.

### Migrator (nginx)

| Input | Emit |
|-------|------|
| `try_files $uri /index.html;` | Supported → flags + `fallback_rewrite="/index.html"` |
| `try_files $uri $uri/ /index.php?$args;` (or `$is_args$args`) | Supported → `try_uri_dir=true`, `fallback_rewrite="/index.php"` + note query preserved |
| `@named`, other vars, intermediate literals | Unsupported / Error (unchanged fail-closed) — SoT `F1-R08`…`F1-R10` in [`F1_RESIDUALS`](../../ROADMAP%20EXYONQ/F1_RESIDUALS.md) |

## Consequences

- SPA and WordPress nginx patterns migrate without ⚠️ for admitted forms.
- Rematch budget stays one hop (no loops).
- Full try_files expressiveness remains out of scope (named locations, arbitrary candidates).
