# ADR-064 — NG-18 body transforms: REOPENED as product compatibility

**Status:** REOPENED — Phase A/B closed; Phase C/D/E implemented (C/D/E differential pending)
**Date:** 2026-10-27
**Depends on:** NGINX importer, response pipeline, proxy cache/Range, FPC
**Contract:** `SEMANTIC_NGINX_SUBSET` — real differential evidence against NGINX on Netcup (amd64) and Oracle (arm64)

## Why the previous decision was wrong

The first version of this ADR declared `sub_filter`, `slice`, `mirror` and `gunzip` ❌ with an "alternative". That protects migrations from silent breakage, but it does **not** provide compatibility: an nginx config using these directives cannot migrate, and `UNSUPPORTED` is a product gap, not parity. The F5 DoD ("cache/headers tables without ❌") cannot be satisfied by documenting the gap.

NG-18 is therefore **product work**. The importer stays fail-closed per directive until that directive's milestone closes; after that, it must emit `Supported` with evidence.

## Scope and contexts

NGINX defines all four in `http`, `server` and `location`. ExyonQ must resolve and validate all three levels, including inheritance:

| Directive | Real behaviour to reproduce | Pipeline stage |
| --- | --- | --- |
| `gunzip` (+`gunzip_buffers`) | Decompress `Content-Encoding: gzip` when the client does not accept gzip; streaming, bounded | Response filter |
| `sub_filter` (+`sub_filter_once`, `sub_filter_types`, `sub_filter_last_modified`) | Streaming replacement, chunk-boundary aware, MIME-scoped, inheritance (parent rules only when the child declares none) | Response filter |
| `mirror` (+`mirror_request_body`) | Async internal subrequest, response discarded, body policy | Request phase |
| `slice` (+`$slice_range`) | Range segmentation, upstream subrequests, cache per segment | Proxy cache + Range |

## Phases and exit criteria

### Phase A — differential contract (current)

- Corpus with the same NGINX config, the same real upstream, the same request, in `http`/`server`/`location`.
- Compare status, headers, body bytes, ranges, cache and error paths against **real NGINX**.
- Evidence on Netcup amd64 + Oracle arm64; no Mac/Docker evidence as canonical.
- Exit: corpus runs green for the directive under work; NGINX oracle captured.

### Phase B — `gunzip`

- Parse `Accept-Encoding` correctly, including `gzip;q=0`.
- Keep gzip for clients that accept it.
- Remove `Content-Encoding`, recalculate framing, bounded streaming buffers, backpressure.
- Respect `gunzip`, `gunzip_buffers`, `http`/`server`/`location` inheritance.
- Never transform `HEAD`; never fake `Content-Length`, `ETag`, `Vary` or `Last-Modified`.
- Exit: differential green on both hosts; importer emits `Supported`; `UNSUPPORTED` removed for `gunzip`.

### Phase C — `sub_filter`

- Multiple rules, case-insensitive matching, variables, MIME types, `once on/off`.
- Streaming matcher: matches split across chunks; keep only a tail of the longest pattern.
- Remove `Last-Modified` by default unless `sub_filter_last_modified on`.
- Test compressed content together with `gunzip` before fixing pipeline order.
- Exit: differential green; importer `Supported`.

### Phase D — `mirror`

- Resolve destination as an internal route, not a free URL.
- Multiple mirrors; mirror failure never changes the main response.
- `mirror_request_body` default `on` (nginx 1.27). With `on` the client body is read before the subrequests are created and reused from the main path's bounded buffer (32 MiB proxy cap, no second copy); with `off` the subrequest carries a well-framed empty body (no inherited `Content-Length`).
- Metrics: started, completed, cancelled, failed.
- Exit: differential green; importer `Supported`.

### Phase E — `slice`

- Requires a real proxy cache: body+metadata on disk with atomic writes; key separates representation/host/query/range; 200 and 206; `Range`, `Content-Range`, `If-Range`, `ETag`, cancellation; protection against user/cookie/auth/Vary mixing.
- `$slice_range` propagation to upstream; cache per segment.
- Exit: differential green; importer `Supported`.

## Current status

| Directive | Runtime | Importer | Evidence |
| --- | --- | --- | --- |
| `gunzip` | Phase B closed; 420/420 local + both Linux hosts | `Supported` | `ng18-gunzip-diff.py`; Netcup amd64 + Oracle arm64 |
| `sub_filter` | Phase C implemented (streaming + importer) | `Supported` | unit + handler + importer tests; differential pending |
| `mirror` | Phase D implemented: background subrequest, `mirror_request_body on` (default) and `off`, query inheritance | `Supported` | `ng18-mirror-body-diff.py` 18/18 vs NGINX 1.27; importer tests |
| `slice` | Phase E implemented: segment cache (`range` key) + upstream Range + assembly | `Supported` | E2E 4 MiB / 4 segments / second GET hit — Netcup amd64 + Oracle arm64 |

## Phase B evidence (2026-10-27)

- Corpus: `scripts/gates/ng18-gunzip-diff.py`, 420 rows across `http`/`server`/`location`/inheritance, `GET`+`HEAD`, AE `identity|gzip|gzip;q=0|*|br, gzip;q=0.5|absent`.
- NGINX oracle: Mac (1.27-alpine), Netcup amd64 (1.27-alpine), Oracle arm64 (1.24.0). 20 rows are `RemoteDisconnected` (invalid gzip), matching on all three hosts.
- ExyonQ local diff: **420/420 OK, stable across two consecutive runs**.
- Linux: **420/420 on Netcup amd64 and Oracle arm64** with the binaries extracted from the `exyonq:ng18-*` images.
- Invalid gzip aborts the body (`BoxError` boundary → Hyper closes the connection), matching NGINX's `RemoteDisconnected` class. The corpus origin is a static NGINX (deterministic headers/framing).
- Importer `gunzip`/`gunzip_buffers` → `Supported`.
- Real container E2E (not smoke): server amd64/arm64 (static, proxy, gunzip, passthrough, invalid-gzip abort) and WordPress amd64/arm64 (install, login, FPC hit, cached nonce POST, logged-in bypass).
- Phase C (`sub_filter`) implemented: `[modules.sub_filter.routes]`, streaming matcher, nginx importer with http/server/location inheritance, handler/import tests. Differential corpus pending.
- Phase D (`mirror`) closed: `[modules.mirror.routes]`, background internal subrequest with anti-recursion guard, `mirror_request_body on|off` (default `on`, matching nginx 1.27), parent query-string inheritance, multiple destinations, importer `http`/`server`/`location` inheritance and `mirror off`. Differential `scripts/gates/ng18-mirror-body-diff.py`: 18/18 (on/off/multi) against real NGINX 1.27 + real origin + raw sink; body hashes, length, method, target and Content-Type identical; slow mirror does not delay the main response; client abort and over-limit fire no mirror. The raw sink also shows the nginx `off` footgun (inherited `Content-Length` with no body); ExyonQ always sends a well-framed empty body, which is the docs' required mirror-location config.
- Phase E implemented: full-object `Range`/`206`/`If-Range`; `slice` with range-keyed segment cache, upstream `Range` subrequests and assembly. E2E: 4 MiB, 4 segments, second GET 100% cache hit (0 extra ranges) on Netcup amd64 and Oracle arm64. Importer `slice <size>` http/location inheritance. Differential C/D/E pending.
- Next: range-keyed cache + `slice` segmentation, then Phase C/D differential.

## Risks / do not

- Do not call an "alternative" something that does not reproduce the semantics.
- Do not do a global proxy refactor or touch pooling/timeouts/P99 while adding filters.
- Do not buffer whole bodies for `sub_filter` or `gunzip`.
- Do not enable `mirror_request_body on` without a bounded spool.
- Do not start `slice` before proxy cache and Range are proven correct.

## Superseded decision (2026-10-27, historical)

The previous version declared all four ❌ with alternatives (origin/WASM rewrite, sendfile+page cache, upstream/edge tap, identity upstream). That decision is retained only as history; it is not the current contract.
