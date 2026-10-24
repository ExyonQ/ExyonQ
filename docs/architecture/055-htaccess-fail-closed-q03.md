# ADR-055 — `.htaccess` fail-closed for non-applied directives (Q-03 / v0.6 F2)

**Status:** Accepted  
**Date:** 2026-10-18  
**Depends on:** Cap htaccess overlay (rewrite-only), EXY-HTACCESS-* registry  
**Contract:** CONDITIONAL ACCEPT — security/header directives must never publish silently

## Context

QUEJAS_APACHE §2.2 / QUEJAS_OLS §2.1: operators believe `Require all denied` / `Header` / `Expires` in `.htaccess` are enforced. ExyonQ overlay only executes rewrite/redirect/`DirectoryIndex`/`Options -Indexes`. Unknown lines were recorded as `unknown:` diagnostics while the overlay **still published** — silent security gap.

## Decision

### Fail-closed classes

| Class | Code | Directives (case-insensitive) | Publish |
|-------|------|-------------------------------|---------|
| Forbidden (security/auth/ACL) | `EXY-HTACCESS-0003` | `Require`, `Deny`, `Allow`, `Order`, `Satisfy`, `AuthType`, `AuthUserFile`, `AuthName`, `AuthGroupFile`, `AllowOverride`, `php_value`, `php_flag` | **No** — compile error |
| Unsupported (headers/expires/containers) | `EXY-HTACCESS-0002` | `Header`, `ExpiresActive`, `ExpiresByType`, `ExpiresDefault`; container openers `<FilesMatch>`, `<Files>`, `<Directory>`, `<IfModule>`, `<Limit>`, `<LimitExcept>` | **No** — compile error |

Every error line includes **file:line**, stable code, and an **exact alternative** (WAF/`[[route]] access`, app headers, F5 `cache_headers`, or move rule to IR).

### Still silent-ok (v0 overlay)

Executed: `DirectoryIndex`, `Redirect`, `RewriteEngine`, `RewriteRule`/`RewriteCond` (limited), `Options -Indexes`.  
`parsed:` (RedirectMatch, RewriteBase, unsupported Options tokens) remain non-fatal unless listed above.

### Non-goals

- Full Apache `.htaccess` execution  
- Auto-migration of `Require` into WAF rules  
- Changing Cap040 drain / Q-05 WebSocket guarantees  

## Consequences

- `compile_and_publish` retains prior overlay on these errors (`EXY-HTACCESS-0005` path).  
- `exyonqctl htaccess explain` classifies the same names as FORBIDDEN/UNSUPPORTED.  
- Residual broad overlay: `F1-R05` (DEFER_LATER) remains for full Header/Expires/Auth product support.
