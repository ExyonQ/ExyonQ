# ADR-065 — nginx `proxy_cache_key` equivalence (NG-15)

**Status:** Accepted
**Date:** 2026-10-27
**Depends on:** NG-15 (`[full_page_cache.key]` / `.bypass`), FPC WC2B/WC2C
**Contract:** documentation of the implemented mapping; no runtime change

## Context

NG-15 requires a documented equivalence with nginx `proxy_cache_key`. ExyonQ does not expose an nginx-style key string: the FPC key is a structured tuple compiled per route.

## Mapping

| nginx | ExyonQ | Notes |
| --- | --- | --- |
| `proxy_cache_key $scheme$host$request_uri` | `CacheKeyParts { site_id, namespace, backend_id, runtime_generation, route_idx, method, scheme, host, path, query, content_encoding, vary }` | `path` is canonicalized; `query` carries only `[full_page_cache.key] query` params, sorted |
| `proxy_cache_key …$cookie_x` | `[full_page_cache.key] cookies = ["x"]` | Only listed cookies pass the gate; their values enter `vary` |
| `proxy_cache_key …$http_accept_language` | `[full_page_cache.key] headers = ["accept-language"]` | A response `Vary` is storable only if every listed header is in `key.headers` |
| `proxy_cache_bypass $cookie_nocache $arg_nocache` | `[full_page_cache.bypass] cookies = ["nocache"]` / `headers = [...]` | Query bypass is automatic: any non-keyed query param bypasses |
| `proxy_no_cache $http_pragma` | response store gate (`Cache-Control: no-store/private/no-cache`, `Set-Cookie`, non-allowlisted `Vary`, streaming, hop-by-hop) | Fail closed |
| `proxy_cache_methods GET HEAD` | GET fills; HEAD never stores but reuses GET | Method gate |
| `proxy_cache_valid 200 30m` | `[full_page_cache] default_ttl_seconds` + `max_ttl_seconds` (clamped) | `Cache-Control: s-maxage`/`max-age` also clamps |
| `proxy_cache_use_stale error` | `[full_page_cache] stale_if_error_secs` | Serves expired entries on backend 5xx with `X-Cache: stale` |
| `proxy_cache_path … keys_zone=…` | `[full_page_cache] max_entries` / `max_total_bytes` / `max_object_bytes` / `namespace` | In-memory L1 |
| `proxy_cache_purge` | `exyonqctl purge url\|tag <site_id> …` | Tags come from `[[route]] cache_tags` |
| `add_header X-Cache $upstream_cache_status` | built-in `X-Cache: hit\|miss\|bypass\|stale` + `X-Cache-Reason` | Q-04 |
| `proxy_cache_lock` | single-flight coalescing (cold key) | Documented limitation: only cold-key coalescing |
| `proxy_cache_background_update` | **not implemented** | No `stale-while-revalidate`; only `stale-if-error` |

## Operator recipe

```toml
[full_page_cache]
enabled = true
namespace = 4
default_ttl_seconds = 300
max_ttl_seconds = 3600
stale_if_error_secs = 600

[full_page_cache.key]
query = ["page"]
cookies = ["wpml"]
headers = ["accept-language"]

[full_page_cache.bypass]
cookies = ["nocache"]
headers = ["x-no-cache"]
```

Diagnose with `exyonqctl cache explain --config PATH URL` and `X-Cache`/`X-Cache-Reason`.

## Non-goals

- Accepting a raw nginx key string; keys stay structured.
- `stale-while-revalidate` / `proxy_cache_background_update`.
- Byte-for-byte nginx `$request_uri` semantics: ExyonQ canonicalizes the path and drops non-keyed query params.
