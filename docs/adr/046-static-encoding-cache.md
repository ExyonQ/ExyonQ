# ADR 046 — Static encoding cache (gzip first; Cap067-compatible)

```text
ADR = 046
TITLE = Static encoding cache (gzip first; Cap067-compatible)
STATUS = ACCEPTED — Cap067 MVP IMPLEMENTED (2026-09-22); CFD Static FOLLOWUP
DATE = 2026-09-22
PRODUCT_MUTATION = YES (admission + Cap067 MVP runtime WIP)
PARENT = ADR-044 (CFD Static) + Cap067 product static + Cap022/023 negotiate
IMPLEMENTATION_FROM_THIS_ADR_ALONE = SUPERSEDED — follow-up WIP authorized below
CONTRACT = CONDITIONAL ACCEPT (exyonq-architecture-contract 2026-09-22)
```

## Implementation WIP (authorized 2026-09-22)

MVP Option A (disk cache) is **authorized for Cap067** under this follow-up:

| Surface | Choice |
| --- | --- |
| Plane | Cap067 / `exyonq-mod-static` only (no Cap067↔CFD handoff) |
| Storage | Managed dir `[static.encoding_cache]` (default OFF) |
| Miss | A1 sync compress-once (`flate2` gzip level 6; optional brotli) + atomic tmp+rename |
| Serve | sendfile coded FD + `Content-Encoding` + `Vary: Accept-Encoding` |
| Kill | IR `enabled=false` or `EXYONQ_STATIC_ENCODING_CACHE=0` |
| Out of scope | Cap034 proxy TE |
| Brotli static | Cap067 path supports `brotli=true` (same cache dir, coding suffix `-br`) |
| CFD Static | FOLLOWUP — native `BackendKind::Static` may later open coded objects under Cap004; **no** Cap067 handoff (ADR-044 F/G REJECT) |

### CFD Static FOLLOWUP (not Cap067 remingle)

When CFD Static gains precompressed:

1. Reuse the same cache identity schema `(dev,ino,mtime,size,coding,level)`.
2. Open coded FD via CFD Cap004 / `PlatformStaticIo` only inside the mio shard.
3. Do **not** call `exyonq-mod-static` / Cap067 sendfile from CFD.
4. Until then, product P9 evidence remains Cap067-plane only.

Netcup P9 before/after + architecture-compiler remain merge gates for claiming
P9 competitiveness; this WIP ships the Cap067 product path and unit tests.

## Context

P9 (`/site/64k.txt`, `Accept-Encoding: gzip`) on Netcup DEV (`p9-gzip-profile-20260921T220046Z`,
`p9-cl824-20260922T135045Z`) shows:

| Server | Model | med RPS | gzip CL |
| --- | --- | --- | --- |
| OLS | static gzip cache + sendfile | ~214k | **412** (= zlib gzip-6 of payload) |
| nginx | on-the-fly zlib level 1 | ~36k | **650** |
| ExyonQ | per-request `flate2::GzEncoder` level 1 | ~49k | **824** (= flate2/miniz gzip-1; valid gunzip) |
| ExyonQ identity (no AE) | Hyper + modules, Cap067 OFF | ~70k | 65536 |

OLS config (`benchmarks/configs/openlitespeed/httpd_config.conf`):
`gzipAutoUpdateStatic 1`, `gzipStaticCompressLevel 6`, `useSendfile 1`.
LiteSpeed docs: compress once into Static Cache Directory; hot path serves coded bytes.

ExyonQ today (`modules/compression`): Hyper ResponseFilter buffers the body and
re-encodes every request. Cap067 sendfile does not negotiate `Accept-Encoding`,
so Cap067 must be OFF for CE:gzip to apply. CFD Static admission marks
`precompressed = OUT_OF_SCOPE` (amended by this ADR to FOLLOWUP).

Tuning zlib level alone cannot close a ~4× RPS gap while identity ExyonQ (~70k)
remains far below OLS (~214k).

### CL=824 closed

Netcup dump gunzips to the identical 64 KiB payload. Local
`flate2::write::GzEncoder` at level 1 yields **exactly 824** bytes (`xfl=4`,
`os=255`). Not double-compression and not a framing bug — compressor backend
differs from nginx/zlib (650) and OLS/zlib-6 (412).

## Options considered

| ID | Option | RPS path | Cap067 | Complexity | Verdict |
| --- | --- | --- | --- | --- | --- |
| A | Disk sidecar / managed cache dir + sendfile of coded file | high | yes | med | **ACCEPT MVP** |
| B | In-memory cache keyed by file identity + coding | high (small working set) | partial (no zero-copy) | med | ACCEPT phase-1.5 / hot-file |
| C | Status quo dynamic filter only | low | Cap067 OFF for gzip | low | REJECT as end state for P9 competitiveness |
| D | Generic encoding policy engine | unknown | n/a | high | **REJECT** |
| E | Cap067 ↔ CFD per-request handoff for gzip | — | dual plane | high | **REJECT** (ADR-044 F/G) |
| F | Raise flate2 level / micro-opts only | small | still hot-path zlib | low | REJECT as sole fix |

## Decision

1. **Admit** a product capability: **static encoding cache** — compress (or load)
   coded bytes **once** per file identity + content-coding, then serve those bytes
   with `Content-Encoding` + `Vary: Accept-Encoding` without re-entering zlib on
   the hot path.
2. **Primary plane for P9 evidence:** Cap067 / `exyonq-mod-static` (product static).
   CFD Static (`BackendKind::Static`) is **FOLLOWUP**; amend ADR-044 / admission
   binder `precompressed: OUT_OF_SCOPE` → `FOLLOWUP (ADR-046)`.
3. **MVP coding:** gzip. Brotli static cache = phase 2. Zstd static cache =
   optional later or OUT until measured.
4. **MVP storage:** **Option A** (managed cache directory or controlled sidecar
   under docroot policy). Option B allowed as additive hot-cache with hard
   RSS/entry caps; must not be unbounded.
5. **Cap067 interaction:** on cache hit, prefer **sendfile of the coded object
   FD** (same FSM family as identity static). Cap067 eligibility must become
   **encoding-aware** (may sendfile coded bytes when AE matches; must not
   sendfile identity when client asked gzip and a valid coded object exists).
6. **Negotiate SSOT:** reuse Cap022/023 preference / q-value rules. One decision;
   no second preference table. If response already has `Content-Encoding`, do not
   double-encode (existing compression-module invariant).
7. **Miss policy (MVP — pick one, deterministic):**
   - **A1 (preferred):** synchronous compress-once on miss, publish to cache,
     then serve coded bytes (may block one request; bound CPU and size); or
   - **A2:** serve identity or dynamic filter until off-path update completes
     (document visibility window; no silent races).
8. **Kill switch:** config/default **off** ⇒ behavior identical to today
   (dynamic `exyonq-compression` only; Cap067 unchanged for identity).
9. **Non-goals:** Cap034 proxy TE framing; autoindex; alias expansion; H2/H3;
   generic policy engine; making compression mandatory for baseline static;
   CFD↔Cap067 remingle.
10. **Implementation** requires a follow-up WIP with Netcup P9 before/after,
    protector matrix if Cap067 paths change, security review of cache
    containment, and `exyonq-architecture-compiler` before merge of runtime.

## Feature admission (106)

| # | Question | Answer |
| --- | --- | --- |
| 1 | Problem | Hot-path zlib every static gzip request |
| 2 | Metric | P9 RPS / CPU / p95–p99 on Netcup; CL/coding parity as oracle |
| 3 | Competitor | OLS `gzipAutoUpdateStatic` |
| 4 | Outside core? | Yes — Cap067 / mod-static (+ optional compress off-path) |
| 5 | Complexity | One concern: static encoding cache + serve coded bytes |
| 6 | Kill if fails | Config off; delete cache path; no Cap034 fallout |

## Consequences

- P9 methodology remains honest: Official publish still requires sequential
  `bench compare` + verify; DEV KEEP ≠ marketing.
- Dynamic/proxy bodies still use `exyonq-compression` unless separately designed.
- Operators may see larger on-disk cache; must be rooted and bounded.
- Fair rival compare may optionally enable ExyonQ static cache when measuring
  against OLS static gzip (document in scenario notes when implemented).

## Invariants

```text
STATIC_ENCODING_CACHE_DEFAULT = OFF
HOT_PATH_ZLIB_ON_STATIC_CACHE_HIT = FORBIDDEN
DOUBLE_CONTENT_ENCODING = FORBIDDEN
CAP034_PROXY_TE_FRAMING = UNCHANGED
CAP067_CFD_PER_REQUEST_HANDOFF = FORBIDDEN
GENERIC_ENCODING_POLICY_ENGINE = FORBIDDEN
CACHE_IDENTITY = path + (dev,ino|mtime) + size + coding (+ level)
CACHE_UNBOUNDED_GROWTH = FORBIDDEN
IMPLEMENTATION_FROM_THIS_ADR_ALONE = SUPERSEDED_BY_WIP_NOTE
DEV_BENCH ≠ OFFICIAL_PUBLISH
```

## Evidence references (DEV)

```text
p9-gzip-profile-20260921T220046Z   # RPS medians ExyonQ/nginx/OLS
p9-cl824-20260922T135045Z          # body dump; flate2-1 = 824 closed
```
