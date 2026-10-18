# ADR-047 — Route match: `regex` / `suffix` (NG-01 / v0.5 F1)

**Status:** Accepted
**Date:** 2026-10-17
**Roadmap:** F1 / v0.5 NG-01
**Public contract:** [docs/config/route-match.md](../config/route-match.md)

## Context

Cap033 ranks routes by host (exact > wildcard > hostless) then longest directory-prefix on `match.path`. NGINX `location ~` / `~*` and Apache path regex cannot be expressed. COMPAT_NGINX §4.1 and ROADMAP F1 require `match.regex` / `match.suffix` with a documented ranking that is **not** a silent clone of nginx (`^~` / first-regex-wins).

## Decision

### IR (`RouteMatch`)

Exactly **one** of:

| Kind | Fields | Semantics |
|------|--------|-----------|
| Prefix | non-empty `path` starting with `/` | Cap033 directory-prefix (unchanged) |
| Suffix | non-empty `suffix` (no `/` required); `path` empty | literal `path.ends_with(suffix)` |
| Regex | non-empty `regex`; `path` empty | Rust `regex` crate, case-sensitive (`~` style) |

`host` remains optional and orthogonal. Mixing kinds → **fail closed** at config load. Empty match → fail closed.

Regex load bounds: pattern length ≤ 256 bytes; must compile; reject empty. No capture application in NG-01 (NG-02).

### Ranking (total order)

Among routes that **match** the request:

1. `host_rank`: exact named host (2) > `*.wildcard` (1) > hostless (0) — Cap033 unchanged
2. `kind_rank`: regex (3) = suffix (3) > prefix (1) > catch-all prefix `/` (0)
3. `pattern_len`: longer regex/suffix/path string wins within the same kind
4. **Tie:** identical `(host_norm, kind, pattern_key)` at plan build → **fail closed** (duplicate identity). Distinct patterns that somehow tie after (1–3) → prefer **lower original route index** (stable, documented; not nginx-first-regex).

ExyonQ does **not** claim nginx `^~` or “first matching regex in file order” as the product rule.

### Compilation

- Validate XOR + regex syntax in `exyonq-config-ir` at load.
- Compile `Regex` once in `RouteIndex::try_new` / plan build. **Never** on the request path.
- Suffix stays literal `ends_with` (not regex).

### CFD

`exyonq-cfd-gen` `RouteTable` remains **prefix-only Cap033** in this slice. Documented deferral: CFD is not the product HTTP router for classic `exyonq serve`. Residual SoT: [`F1_RESIDUALS`](../../ROADMAP%20EXYONQ/F1_RESIDUALS.md) `F1-R01`/`F1-R02`.

## Consequences

- Existing TOML with only `match.path` unchanged.
- Hot path: linear scan; each regex candidate pays `Regex::is_match` after host filter.
- Migrator mapping of `location ~` is NG-01 follow-up (compat), not blocking runtime NG-01.
