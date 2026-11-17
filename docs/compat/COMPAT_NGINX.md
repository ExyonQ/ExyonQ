# COMPAT — NGINX → ExyonQ (0.9.x)

<!-- compat-matrix
source=nginx
exact=79
approximated=13
unsupported=5
directives=21
p1_open=0
p2_open=0
-->

Measured, not asserted: the rows below come from running
`exyonqctl config migrate-nginx` over the five real configurations in
`tests/fixtures/migrations/` (WordPress, Laravel, SPA, Home Assistant, n8n) and
reading the compatibility report of each one. `scripts/gates/compat-matrices-gate.sh`
re-measures the same corpus on every run and fails if this block disagrees with
the numbers above, if an approximated or refused directive is missing from
`tests/fixtures/compat-matrix/gaps.txt`, or if the block drops a metric.

Counts are **entries in the reports**, so one directive can appear several times
(server scope, location scope, inheritance) and a directive can be exact in one
place and lossy in another.

| Directive | exact | approximated | refused | Verdict | Gap |
|---|---|---|---|---|---|
| `add_header` | 1 | 0 | 2 | ⚠️ partial | Cache-Control denied by ADR-062; cache policy lives in `[cache]`/`expires` |
| `client_max_body_size` | 0 | 0 | 1 | ❌ refused | Route body budget covers the cap |
| `deny` | 1 | 0 | 0 | ✅ exact | — |
| `expires` | 0 | 2 | 0 | ⚠️ partial | Positive `max_age` only; `expires off`/negative have no IR form |
| `fastcgi_pass` | 2 | 0 | 0 | ✅ exact | — |
| `include` | 2 | 0 | 0 | ✅ exact | Expanded (e.g. `fastcgi_params`) |
| `index` | 12 | 4 | 0 | ⚠️ partial | Only the first index reaches `route.index`; the runtime resolves one index |
| `keepalive` | 0 | 0 | 1 | ❌ refused | IR pool connection reuse |
| `listen` | 6 | 0 | 0 | ✅ exact | `ssl` is served by `[server.tls]` (mapped 2026-11-16) |
| `location` | 18 | 2 | 0 | ⚠️ partial | `=`/prefix are exact; `~`/`~*` precedence under a prefix is a documented differential |
| `map` | 2 | 0 | 0 | ✅ exact | `[[map]]` with the ADR-051 source allowlist |
| `proxy_pass` | 2 | 0 | 0 | ✅ exact | — |
| `proxy_set_header` | 6 | 0 | 1 | ⚠️ partial | `X-Forwarded-For $proxy_add_x_forwarded_for` is runtime-managed (2 ignored-safe); `Upgrade`/`Connection` are hop-by-hop |
| `return` | 1 | 0 | 0 | ✅ exact | Literal body mapped to `route.respond` and served by the handler; a body containing `$` stays lossy (IR bodies are not interpolated) |
| `root` | 14 | 0 | 0 | ✅ exact | — |
| `server` | 1 | 0 | 0 | ✅ exact | — |
| `server_name` | 5 | 0 | 0 | ✅ exact | — |
| `ssl_certificate` | 1 | 0 | 0 | ✅ exact | `[server.tls].cert` (mapped 2026-11-16) |
| `ssl_certificate_key` | 1 | 0 | 0 | ✅ exact | `[server.tls].key` (mapped 2026-11-16) |
| `try_files` | 3 | 5 | 0 | ⚠️ partial | Variable forms map to `front_controller`; terminal `=404` is the default 404 |
| `upstream` | 1 | 0 | 0 | ✅ exact | — |

## Open gaps

`P1_OPEN=0 P2_OPEN=0`. One P2 remains, with a declared workaround:

- **`location` — P2, workaround.** NGINX evaluates `~`/`~*` regex locations
  before a prefix match and orders `^~` prefixes; ExyonQ evaluates prefix,
  suffix and exact matches in a fixed order. Declaring the same set as explicit
  routes reproduces the intent, but a migrated config alone can route a URI to
  the prefix route where nginx would have used the regex block.
- The approximated entries left are P3 with a documented workaround
  (`expires`, `index`, `try_files`) and the five refusals are P3 as well.

`return <status> "<body>"` is no longer a gap: the IR carries a literal
response (`route.respond` with `status`, `body` and optional `content_type`),
`exyonq validate` accepts it, and the handler serves it before static or proxy
resolution, so `/healthz` answers `200` with body `ok`. `return` with a body that
contains nginx variables is still reported lossy and not mapped: an IR literal
body is served verbatim, so interpolating would ship different text. `return 204`
and `return 444` stay refused (a bodyless literal response has no construct yet;
444 has no equivalent).

## Not covered by this matrix

Runtime behaviour after migration: identical responses to nginx are the subject
of the E2E suites, not of this importer taxonomy. Servers nginx/Apache/OLS are
not started here.
