# WAF rulesets, detectors, and exclusions

Authority: `crates/exyonq-waf/src/detect/mod.rs`, `exclusion.rs`, `compile.rs`,
`config/ir/src/waf.rs`.

## Ruleset semantics

- `[[waf.ruleset]]` entries are named collections of optional custom rules.
- `id` must be unique; duplicates are rejected at config load.
- `enabled = false` skips that ruleset’s custom rules.
- Built-in detectors are compiled from the default pack (or from optional
  `[waf.ruleset.builtins]` toggles on a ruleset). Last ruleset that supplies
  `builtins` wins at composition time.
- Custom rule IDs must be unique across all rulesets (reject on duplicate).
- Signature custom rules may not use `challenge` / `rate_limit` actions
  (abuse-only actions).

Reload: rulesets are compiled into an immutable snapshot during Cap013 prepare
and swapped on commit.

## Built-in detector toggles

Optional under a ruleset:

```toml
[[waf.ruleset]]
id = "default"

[waf.ruleset.builtins]
sql_injection = "block"      # off | log | block
xss = "block"
path_traversal = "block"
command_injection = "block"
header_anomaly = "log"
bot_ua = "log"

[waf.ruleset.builtins.ip_filter]
enabled = true
whitelist = ["10.0.0.0/8"]
blacklist = ["203.0.113.0/24"]
```

### `[waf.ip]` — global client-IP allow/deny list

The same detector (`EXY-IP-1001` / `EXY-IP-1002`) is configurable at product
level, so an operator does not need a ruleset or an external firewall:

```toml
[waf.ip]
whitelist = ["198.51.100.0/24"]
blacklist = [
  "203.0.113.0/24",
  { ip = "192.0.2.10", until = "2026-11-21T08:30:00Z", reason = "repeated 404 scanning" },
]
```

| Field | Meaning |
| --- | --- |
| entry string | IP or CIDR, no expiry, no reason |
| `ip` | IP or CIDR of an expanded entry |
| `until` | RFC 3339 UTC instant after which the entry stops applying (`Z` or `±HH:MM`) |
| `reason` | Operator note recorded with the violation |

Semantics:

- A declared list is active policy: `[waf.ip]` with entries enables the filter
  even when no `[waf.ruleset.builtins.ip_filter]` is declared. `[waf.ip]`
  entries are unioned with the ruleset lists; blacklist still wins.
- `enabled = false` (whole `[waf]`) turns the filter off; there is no second
  switch on `[waf.ip]`.
- Expiry: an expired blacklist entry stops blocking. A non-empty whitelist is
  **fail-closed**: an expired allow entry stops allowing, and a client outside
  the list is still rejected (`EXY-IP-1002`).
- The violation log line carries `ip_entry=Some(WafIpEntryEvidence { list: …, entry: …, reason: … })`,
  so the matched list entry and the operator reason are auditable.
- Invalid IP/CIDR, a non-RFC 3339 `until` or an empty `reason` fail
  `exyonq validate` (fail-closed).
- Metrics: `exyonq_waf_ip_filter_entries{list,state}` (gauge, `active`/
  `expired` at scrape) and `exyonq_waf_ip_entry_hits_total{list,entry}`
  (blacklist matches; capped like the rule series, overflow counted in
  `exyonq_waf_ip_entry_hits_labeled_dropped_total`).
- The CFD projection carries only the network text: TTL and reasons stay
  host-side.

Custom-rule `target = "client_ip"` is still **not** implemented; the issue's
list scope is covered by `[waf.ip]` and `builtins.ip_filter`.

### Defaults when enabled and no builtins table

| Detector | Default action |
| --- | --- |
| SQL injection | Block |
| XSS | Block |
| Path traversal | Block |
| Command injection | Block |
| Header anomaly | Log |
| Bot UA | Log |
| IP filter | Off (empty) |

## Built-in rule IDs (current implementation)

| Rule ID | Family | Phase / input |
| --- | --- | --- |
| `EXY-SQL-1001` | SQLi | path/query/body (normalized) |
| `EXY-SQL-1002` | SQLi | path/query/body |
| `EXY-SQL-1003` | SQLi | path/query/body |
| `EXY-XSS-1001` | XSS | path/query/body |
| `EXY-XSS-1002` | XSS | path/query/body |
| `EXY-XSS-1003` | XSS | path/query/body |
| `EXY-PATH-1001` | Path traversal | path/query/body |
| `EXY-PATH-1002` | Sensitive paths | path/query/body |
| `EXY-CMD-1001` | Command injection | path/query/body |
| `EXY-CMD-1002` | Shell binaries | path/query/body |
| `EXY-HDR-1001` | Header anomaly | missing/empty User-Agent |
| `EXY-HDR-1002` | Header anomaly | header value > 8 KiB |
| `EXY-BOT-1001` | Bot UA | User-Agent patterns |
| `EXY-BOT-1002` | Bot UA | User-Agent patterns |
| `EXY-IP-1001` | IP filter | client IP blacklisted |
| `EXY-IP-1002` | IP filter | client IP not in whitelist (when whitelist non-empty) |
| `EXY-LIMIT-BODY-001` | Inspection limit | body budget exhausted under Block / LogAndAllow |

Known good-bot substrings (bot detector allow): `googlebot`, `bingbot`, `slurp`,
`duckduckbot`, `facebookexternalhit`, `twitterbot`.

Normalization: URL-decode within a bounded budget (including double-encoding)
before pattern match. Body scan uses lossy UTF-8 so binary bodies are not
silently skipped.

## Exclusions

```toml
[[waf.exclusion]]
host = "example.com"
path_prefix = "/upload"
method = "POST"
rule_ids = ["EXY-XSS-1001"]
```

| Selector | Behavior |
| --- | --- |
| `host` | When set, must match request host |
| `path_prefix` | Matches exact path or `prefix + "/" + remainder` only |
| `method` | When set, must match |
| `rule_ids` | Required non-empty allowlist of rule IDs to suppress |

Properties:

- Scoped: suppresses listed rules only; does **not** disable the WAF.
- Empty `rule_ids` rejected at config; at runtime empty lists match nothing.
- Generation-bound via compiled snapshot.
- Not a generic bypass switch.

## Multiple matching rules

Hits are aggregated; signature `Block` wins over weaker signature actions when
enforce applies. Exclusions remove individual rule hits before aggregation.
