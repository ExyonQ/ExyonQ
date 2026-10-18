# ADR-057 — `X-Cache` observability (Q-04 / v0.6 F2)

**Status:** Accepted
**Date:** 2026-10-18
**Depends on:** FPC WC2B/WC2C (`fpc_lookup` / `fpc_store`)
**Contract:** CONDITIONAL ACCEPT — emit `X-Cache` on FPC outcomes; stats via Prometheus; offline explain

## Context

QUEJAS_OLS §2.7 / comparative reports: ExyonQ FPC leaves no client-visible trace (`Age` stripped; no `X-Cache`).

## Decision

| Outcome | Headers | Where |
|---------|---------|-------|
| L1 HIT | `X-Cache: hit` | `fpc_lookup::served_entry_to_response` |
| Eligible MISS (origin fill) | `X-Cache: miss` | `static_dispatch_fpc_fill` / `fastcgi_dispatch_fpc_fill` |
| FPC enabled but skipped | `X-Cache: bypass` + `X-Cache-Reason: <token>` | gate `Bypass` → handler `maybe_apply_bypass` |
| FPC disabled | *(no X-Cache)* | gate `Skip` |

Bypass reason tokens come from `BypassReason::as_str` (`method`, `cookie_denied`, `wp_admin`, …) or gate labels (`namespace`, `no_site`, `lookup_error`).

Strip any origin-supplied `X-Cache` / `X-Cache-Reason` before applying ExyonQ’s value.

### CLI

- `exyonqctl cache stats [--url http://host/metrics]` — scrape `exyonq_fpc_*` (appender registered at serve start).
- `exyonqctl cache explain --config PATH URL` — offline `fpc_request_evaluate` decision.

### Prometheus

`exyonq_cache::append_fpc_prometheus` registered from `exyonq serve` after observability runtime.

## Non-goals

- Changing FPC eligibility / TTL / keying
- Full CDN `Cache-Status` RFC parity
- Live L1 contents dump in `cache explain`
