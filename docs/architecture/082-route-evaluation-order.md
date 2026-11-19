# Route evaluation order and the `server.routes` allowlist

Two rules decide which `[[route]]` answers a request, and both surprised
operators in 0.9.1: a route must be listed by a server to exist at all, and the
most specific matcher wins regardless of list order.

## 1. `server.routes` is an explicit allowlist

A `[[route]]` block is only reachable through a `[[server]]` that names it:

```toml
[[server]]
listen = "0.0.0.0:443"
routes = ["site", "ret_verification"]   # both names must appear here
```

A route defined in the root file or in an `include` fragment but listed by no
server never matches, even though the config validates and appears in
`dump-ir`. Since 0.9.3 `exyonqctl config lint|test|validate` reports it:

```text
warning[EXY-CONFIG-0016]: route `ret_verification` is defined in include
  `/etc/exyonq/routes/verification.toml` but is not listed in any
  [[server]].routes, so it never matches; add "ret_verification" to
  server.routes or remove the route
```

The diagnostic names the file that defined the route — the include fragment
when there is one — and is emitted without `--warn` because it is the only
signal that the route is inactive. `--strict` turns it into a failure for CI.

An `include` path that does not resolve is a hard error
(`error[EXY-CONFIG-0006]: include file not found: <resolved path> --> <root
config>`); it is never skipped.

## 2. Specificity order

`RouteIndex::best_match` ranks the routes that match the request, highest first:

| Rank | Matcher | Meaning |
| --- | --- | --- |
| 1 | port-scoped `match.listen_port` | a route bound to this listener wins over an unscoped one |
| 2 | `match.host` (exact, then `*.suffix`, then none) | host-scoped over hostless |
| 3 | `exact = true` prefix | whole-URI equality (`location =`) |
| 4 | `regex` / `suffix` | pattern rules |
| 5 | `path` prefix | directory prefix (`/api` matches `/api` and `/api/...`) |
| 6 | `path = "/"` | catch-all |

Within the same rank the longer pattern wins, and ties fall to the route listed
first in `server.routes`.

`exact` outranking `regex` is what makes a single-file exception possible
against a global pattern rule:

```toml
# Redirect every .html page …
[[route]]
name = "html_pages"
match = { regex = "^/(.+)\\.html$" }
redirect = { status = 301, location = "/index.php?p=$1" }

# … except one verification file served from disk.
[[route]]
name = "ret_verification"
match = { path = "/verification-file.html", exact = true }
root = "/srv/verification"
```

`GET /verification-file.html` is served from `/srv/verification`; any other
`.html` path is redirected. Before 0.9.3 the exact route ranked as a plain
prefix, so the generic regex won even when the exact route was listed first, and
the only workaround was a `flag = "break"` rewrite.

`match.regex` also participates in rewrites: an exact route is matched after the
structural rewrite is evaluated, so a `rewrite` on the catch-all route still
applies to the paths the exact route does not claim.

## 3. `dump-ir` shows the effective match

`exyonqctl config dump-ir <path>` re-emits every set arm of `match` — `path`,
`host`, `suffix`, `regex`, `header`, `header_value`, `literal`, `exact` and
`listen_port` — so the document can be read back into the same config. Before
0.9.3 a `match.regex` route was printed as `match = { path = "" }` and `exact`
was dropped, which made route debugging unreliable.
