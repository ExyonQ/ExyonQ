# ADR-059 — `config explain` value provenance (Q-06 / v0.6 F2)

**Status:** Accepted
**Date:** 2026-10-18
**Depends on:** `exyonq-config-merge` includes (config_version = 2), P1.4 lexicon explain
**Contract:** CONDITIONAL ACCEPT — show effective value + origin (file:line) + precedence; lexicon mode unchanged

## Context

QUEJAS_OLS §2.2 / QUEJAS_APACHE §2.3: layered includes make operators ask “where did this effective value come from?”. Existing `exyonqctl config explain <term>` is a **public lexicon** (P1.4-WS3), not provenance.

## Decision

1. **Two modes on the same subcommand**
   - Lexicon: `exyonqctl config explain <directive>` (unchanged catalog).
   - Provenance: `exyonqctl config explain --path FILE --pointer KEY`.
2. **Pointer grammar (v1)**
   - Dotted tables: `full_page_cache.enabled`, `modules.metrics.enabled`.
   - Named array-of-tables: `route[name=main].root`, `upstream[name=app].target`.
   - Indexed server (order-stable from merge): `server[0].listen`.
3. **Effective value** comes from `load_with_includes` → validated `AppConfig` (same as lint/reload).
4. **Origin** is the contributing TOML assignment (1-based line). Include fragments are append-only for named `route` / `upstream` / `fcgi_pool`; root owns scalars and ordered `[[server]]`.
5. **Precedence** lists layers low→high: `default` (IR serde defaults) → `root` → each `include` that could contribute. The winning layer is marked `winner = true`.
6. **Non-goals:** full JSON-pointer RFC, `.exy` Serverfile spans, discovery-overlay JSON as a layer, live daemon memory dump.

## Consequences

- Operators debug include chains without reading every fragment by hand.
- Lexicon and provenance share one UX entrypoint; scripts should pass `--path` + `--pointer` explicitly.
