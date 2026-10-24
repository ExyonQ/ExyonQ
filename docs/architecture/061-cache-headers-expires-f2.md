# ADR-061 — Route `cache_headers` from Apache ExpiresByType (F2 transversal)

**Status:** Accepted  
**Date:** 2026-10-18  
**Depends on:** Apache migrator (`exyonq-compat-apache`), static dispatch  
**Contract:** CONDITIONAL ACCEPT — migrate ExpiresByType → `max_age` per MIME; apply `Cache-Control` on static responses

## Context

COMPAT_APACHE §4.2 / F2 transversal / adelanto NG-17: operators need `ExpiresByType` to become ExyonQ IR without waiting for full F5 header product.

## Decision

1. **IR:** `[[route]].cache_headers.by_mime."<mime>" = { max_age = N, immutable = false }`  
   - `max_age`: seconds, validated `1..=315360000`  
   - mime key `"*"` = ExpiresDefault fallback  
2. **Migrator:** `ExpiresActive On` + `ExpiresByType` / `ExpiresDefault` → map on DocumentRoot static route (`vhN_site`) and Alias root routes.  
   Supported duration forms: `A####` and `access plus N year|month|week|day|hour|minute|second(s)` (month=30d, year=365d).  
   Unsupported forms (including `M####` / modification) → reject (no silent drop).  
3. **Runtime:** On static HTTP 200, if response has no `Cache-Control` and the matched route’s `by_mime` hits Content-Type (exact, then `*`), set `Cache-Control: public, max-age=N` (+ `, immutable` when set).  
4. **Non-goals:** nginx `expires`, `Header set` / add_header, `.htaccess` Expires execution (still fail-closed ADR-055), FPC TTL coupling.

## Consequences

- Apache assets configs migrate to greppable TOML.  
- Static responses become cacheable by browsers/CDNs without FPC.  
- Full NG-17 remains F5/v0.9.
