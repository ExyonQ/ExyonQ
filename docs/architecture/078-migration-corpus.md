# Migration corpus (Transversal — migradores)

The migrators are only as good as the configs they are asked to read. This
document records the corpus that `scripts/gates/migration-corpus-gate.sh`
replays on every `cargo run -p xtask -- ci`, the taxonomy it expects, and the
directives ExyonQ deliberately does not represent.

## Corpus

`tests/fixtures/migrations/` holds five NGINX configurations that stand for the
deployment families the roadmap names:

| File | What it represents |
|---|---|
| `wordpress.conf` | PHP-FPM, pretty permalinks (`try_files → /index.php`), static cache block, `.ht` deny |
| `laravel.conf` | TLS listener, single front controller, exact `/favicon.ico` location |
| `spa.conf` | history fallback (`try_files → /index.html`), immutable `/assets/`, `return 200` health route |
| `home-assistant.conf` | `map $http_upgrade $connection_upgrade` + websocket proxy headers |
| `n8n.conf` | named `upstream` with `keepalive`, long `proxy_read_timeout`, SSE-friendly proxy |

`fastcgi_params` is the include both PHP configs pull with
`include fastcgi_params;` — the same file a real `/etc/nginx` ships, so the
include is expanded rather than stubbed.

`expectations.txt` is the manifest the gate reads: one line per corpus file with
the `--strict` exit it must produce and the exact set of unsupported directives
(sorted, comma separated, `-` for none).

## What the gate asserts

For every corpus file, through the real `exyonqctl` binary:

1. `config migrate-nginx --check --write` exits 0.
2. The compatibility report says `RUNTIMEPLAN_COMPILE_RESULT = PASS`,
   `ERRORS = 0`, `INVALID_SOURCE_COUNT = 0` and `EXACT_COUNT > 0`.
3. The generated IR passes `config lint`.
4. `--strict` exits with the recorded code. `--strict` is fail-closed on any
   warning, so a realistic config with warnings exits 1 — the corpus proves the
   flag actually fails instead of silently passing.
5. The unsupported directive set equals the manifest exactly.

A new warning, a new corpus file without an entry, or a corpus file the
importer stops parsing fails the gate. Adding an entry to `expectations.txt`
means documenting the reason below.

`--selftest` is the negative control: it builds three throw-away configs and
requires the gate to reject each one — a drifted unsupported set, a config that
cannot be loaded, and a `--strict` expectation that contradicts the warnings.

## Result (2026-11-16, HEAD `bc3fc2d`, macOS arm64, debug binary)

```
$ bash scripts/gates/migration-corpus-gate.sh
PASS wordpress.conf exact=24 unsupported=add_header,client_max_body_size strict=1
PASS laravel.conf exact=18 unsupported=- strict=1
PASS spa.conf exact=11 unsupported=add_header strict=1
PASS home-assistant.conf exact=10 unsupported=proxy_set_header strict=1
PASS n8n.conf exact=8 unsupported=keepalive strict=1
MIGRATION_CORPUS=PASS files=5
RESULT=PASS MIGRATION_CORPUS_GATE

$ bash scripts/gates/migration-corpus-gate.sh --selftest
PASS selftest unexpected-unsupported (gate rejected)
PASS selftest broken-source (gate rejected)
PASS selftest strict-exit (gate rejected)
RESULT=PASS MIGRATION_CORPUS_GATE_SELFTEST
```

Negative control of the real corpus (not the selftest): deleting `add_header`
from the `spa` line of `expectations.txt` produced
`FAIL spa.conf: unsupported=add_header (expectations.txt says -)`,
`MIGRATION_CORPUS=FAIL files=5 failures=1`, exit 1; restoring the manifest
returned to PASS.

## Unsupported directives, and why

- **`client_max_body_size`** (wordpress) — not in the Tier 2B server subset
  (`EXY-IMPORT-0002`). The request-body cap lives in the route/upstream body
  budget, which the importer does not derive from an nginx server-level value.
- **`add_header Cache-Control`** (wordpress, spa) — denied by the ADR-062
  response-header denylist: caching is carried by `[cache]`/`expires` in the IR,
  not by a raw header the runtime would also rewrite.
- **`proxy_set_header`** (home-assistant) — the `Upgrade`/`Connection` pair for
  the websocket upgrade is hop-by-hop (ADR-051); ExyonQ owns those headers in
  the runtime, so the directive is reported and not emitted.
- **`keepalive`** (n8n) — upstream keepalive is not in the Tier 2A subset; the
  pool uses the IR's own connection reuse policy.

## Importer fix required by the corpus

The corpus caught a real defect: `proxy_set_header X-Forwarded-For
$proxy_add_x_forwarded_for` was copied verbatim into `proxy_set_headers`, and
the generated IR then failed validation with `unknown variable
$proxy_add_x_forwarded_for in header template (ADR-051 allowlist / [[map]])`.
The runtime already resolves the outbound chain (`effective_xff`: real_ip trust
plus the inbound `X-Forwarded-For`), so the importer now reports that directive
as `IGNORED_SAFE` — "managed by the runtime" — and leaves it out. Regression
test: `compat/nginx/tests/tier2b.rs::tier2b_proxy_add_x_forwarded_for_is_runtime_managed`.

## Limits

The corpus is an importer plus IR gate: it does not start nginx, Apache or
OpenLiteSpeed, and it does not serve traffic. Whether the migrated config
behaves identically is covered by the E2E suites under `scripts/e2e/`, not
here. Regex-location precedence under a prefix is still a documented NGINX
differential (reported as `LOSSY`), which is why WordPress and Laravel cannot
be called exact.
