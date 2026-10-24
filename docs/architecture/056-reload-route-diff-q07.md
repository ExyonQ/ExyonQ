# ADR-056 — Reload route diff summary (Q-07 / v0.6 F2)

**Status:** Accepted  
**Date:** 2026-10-18  
**Depends on:** P1.4 reload fingerprint / `classify_reload_diff`  
**Contract:** CONDITIONAL ACCEPT — summarize routes added/removed + fingerprint; no DYNAMIC_INPLACE

## Context

QUEJAS_OLS §2.2: operators reload without seeing which routes appeared or disappeared. Fingerprint alone does not name the change.

## Decision

1. **Route identity:** `name@host:kind:pattern` (prefix/suffix/regex).  
2. **`exyonqctl config reload --diff`:** prints `candidate_fingerprint`, `routes_added`, `routes_removed`, `routes_unchanged`.  
3. **Baseline sources (priority):** `--baseline PATH` → `{config}.exyonq-applied-routes.v1.json` sidecar → empty baseline on first apply when sidecar path is known.  
4. **Live `config reload`:** prints the same summary before control `reload`, then writes the sidecar after successful publish.  
5. **Non-goals:** full IR structural diff, listener address change class beyond existing H3/TLS notes, control-plane route list export.

## Consequences

- Sidecar sits next to the daemon-bound config; operators may delete it to reset baseline.  
- In-place edits without sidecar still get a useful first-apply “all added” summary once, then accurate diffs thereafter.
