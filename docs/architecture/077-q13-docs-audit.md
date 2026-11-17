# Docs audit (Q-13)

Q-13 asks for audited docs where every directive has a copyable example. The
audit is enforced by `scripts/gates/q13-docs-audit.sh`, which takes every
fenced `toml` block out of the tracked `docs/` tree, builds the smallest
surrounding config that the block needs and validates the result with the real
`exyonq` binary. An example that the binary rejects is a documentation defect:
the docs must not show a config the product refuses.

## What the gate checks

1. A block with `config_version` is a complete config and is validated as it is.
   If that fails, the block is retried merged into the base config, because a
   complete-looking block may still be a partial example.
2. A block that starts with a table header (`[waf]`, `[[route]]`, …) is appended
   to the base config, to a route scope, to a server scope, and — for TLS
   sub-tables such as `[server.tls.acme]` — as the first `[[server]]` with a
   `[server.tls]` holding `cert`/`key`, which is where the engine honors `acme`
   and `strict_sni`.
3. A block of bare keys (`match = …`, `proxy_set_headers = […]`) only makes
   sense inside the table its prose names; the gate reports it as `SKIP`
   instead of claiming it was verified.

The base config defines the route and upstream names the docs use (`app`,
`site`, `admin`, `api`, `origin`, `api-up`, `tcp-backend`). Renaming one of
those in the docs shows up as a `FAIL` until the base is updated.

## Result

Run on 2026-11-17 against `6e2e4d3` plus the working tree
(`bash scripts/gates/q13-docs-audit.sh`, real `target/release/exyonq`):

```
Q13_FRAGMENTS=26/28 skipped=2 allowlisted=0 failed=0
RESULT=PASS Q13_DOCS_AUDIT
Q13_DOCS_AUDIT_EXIT=0
```

28 blocks: 4 complete configs and 24 fragments; the two `SKIP` blocks are the
bare-key fragments the gate cannot place. The same two doc defects below were
still present at `6e2e4d3` (the gate failed `24/28 failed=2` there) and were
fixed in the docs in the 2026-11-17 increment, so the counts above are the
post-fix measurement:

- `docs/architecture/056-route-access-acl-ap05.md`: the `access` inline table
  spanned several lines, which TOML rejects (`invalid inline table`).
- `docs/waf/rules.md`: the builtin-detector block used
  `[waf.ruleset.builtins]` without declaring the ruleset. `WafConfig.rulesets`
  is an array of tables with a mandatory `id`, so the example needs
  `[[waf.ruleset]]` and `id = "default"` first.

The two `SKIP` blocks remain `docs/architecture/051-variables-map-match-header-ng05.md`
#1 and #2.

## Allowlist

`scripts/gates/q13-docs-allowlist.txt` holds `<path>#<n> <reason>` entries for an
example that is deliberately not a valid standalone or composable config. It is
empty today. The gate prints `Q13_ALLOWLIST_STALE` for entries that no longer
match, so a fixed example cannot keep a stale exemption.

## Limits

The gate validates configuration syntax and semantics with the real validator.
It does not replay the runtime errors against real nginx, Apache or
OpenLiteSpeed servers, and it does not check prose accuracy — only that every
shown example is a config the binary accepts in the scope the docs describe.
