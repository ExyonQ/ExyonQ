# 084 — GeoIP country policy (offline MMDB)

ExyonQ resolves client countries **offline** from a local MMDB file. No request
ever calls a GeoIP web service or DNS resolver, and no database is bundled or
redistributed with the product: operators bring the file they are licensed to
use.

## Provider neutrality

The reader is not tied to a vendor. It opens any `.mmdb` that exposes the
GeoLite2-Country field paths `country.iso_code` (and optionally
`continent.code`), which covers MaxMind GeoLite2/GeoIP2 Country, IP2Location
LITE DB1 and DB-IP Country files. There is deliberately **no `provider` field**:
adding one would pin operators to a single vendor, licensing model and price.

The only database shipped in this repository is the upstream test fixture under
`crates/exyonq-geoip/tests/fixtures/`, used by the unit tests. It is not a
production database and it is not installed by any package.

## Configuration

```toml
[geoip]
enabled = true
database = "/var/lib/GeoIP/GeoLite2-Country.mmdb"
reload_interval = "24h"
on_missing_db = "fail_closed"
```

| Field | Default | Meaning |
| --- | --- | --- |
| `enabled` | `false` | A country policy requires `enabled = true` and a `database`; config validation rejects the combination otherwise. |
| `database` | — | Path to the MMDB. Relative paths resolve against the process working directory. `exyonqctl config test` opens and validates the file. |
| `reload_interval` | `24h` | `30s`, `15m`, `24h`, `7d` (1 s – 30 d). The file is re-opened lazily on the first lookup after the interval; a failed reload keeps the previous database and records the error. |
| `on_missing_db` | `fail_closed` | `fail_closed` rejects the generation when the database cannot be loaded at startup and denies country-aware policies for addresses with no record. `fail_open` logs the startup failure, starts without country data and skips country constraints for those requests. |

## Route access

Country policy reuses the two precedence models that already exist for IP
policy. There is no third model and no separate `access.countries` block.

`access.allow` / `access.deny` accept `country:XX` entries next to IP/CIDR and
`all`:

```toml
[[route]]
name = "spain-first"
match = { path = "/es" }
upstream = "app"
access = { allow = ["country:ES", "country:PT"] }

[[route]]
name = "no-sanctioned"
match = { path = "/app" }
upstream = "app"
access = { deny = ["country:CN", "country:RU"] }
```

- `deny` wins over `allow`.
- A non-empty `allow` list is an allowlist. Entries are ORed: an address passes
  when it matches any allowed IP/CIDR **or** its resolved country is listed.
- The 64-entry `MAX_ACCESS_NETWORKS` ceiling applies to IP/CIDR entries only; a
  country list is at most a few hundred ISO codes and is not a CIDR expansion.
  `MAX_ACCESS_RULES` (128) still caps ordered rules.
- IP exceptions therefore take precedence naturally: an address listed in
  `allow` passes even when its country would be denied only if it does not
  match the `deny` list first — list the exception in a `rules` policy when
  first-match order matters.

Ordered rules accept `country:XX` as a `source` and keep first-match semantics:

```toml
access = {
  rules = [
    { effect = "allow", source = "192.0.2.7" },      # office exception
    { effect = "allow", source = "country:ES" },
    { effect = "deny",  source = "country:CN" },
    { effect = "allow", source = "all" }
  ]
}
```

When `on_missing_db = "fail_closed"`, a country-aware policy rejects a request
whose address has no country (VPN, CGNAT, an outdated or corrupt database), and
a missing or malformed database rejects the generation at startup. With
`fail_open`, a load failure at startup is logged and country constraints are
skipped; IP/method/schedule constraints still apply. Reload failures always keep
the previous database, regardless of the policy.

## Real client address and proxies

Country is resolved from the address ExyonQ uses as the client address. When a
trusted proxy is configured, that is the forwarded address selected by the
existing trusted-proxy / `X-Forwarded-For` rules; otherwise it is the peer
address. Running GeoIP behind a proxy without configuring the trusted proxy list
resolves the **proxy's** country, so the rule either never matches or applies to
the wrong client. This is an operator configuration requirement, not a GeoIP
feature.

## WAF and logs

Custom WAF rules accept `target = "country"` with a `|`/`,`-separated list of
two-letter codes; matching is exact and case-insensitive. A country rule does
not match when the request has no resolved country; `on_missing_db` governs the
access lists, not an individual rule.

Access events carry `country` and `continent` next to `client_ip` and
`request_id` on the Hyper path, the wire proxy path and WAF rejections. Country
is intentionally **not** a metric label (unbounded cardinality); per-country
counts come from access logs.

## Diagnostics

```bash
exyonqctl config test /etc/exyonq/exyonq.toml
exyonqctl config geoip check /etc/exyonq/exyonq.toml --ip 81.2.69.160
```

`config test` compiles the runtime plan, which opens and validates the MMDB.
`config geoip check` prints database type, build epoch, IP version, node count,
reload interval, fail policy and the resolved country/continent without starting
listeners.

## Limits and honesty

- **Accuracy is approximate.** VPNs, proxies, CGNAT, mobile carriers and stale
  databases defeat country lookups, and a country deny list can block legitimate
  users. Keep IP exceptions for known partners and monitor `403` rates.
- **Licensing.** GeoLite2 requires an account and attribution; IP2Location LITE
  is free with attribution. ExyonQ ships no database and does not accept one in
  the repository. The upstream test fixture is the only file present.
- **Scope of v1.** Country and continent only. No city, ASN, ISP, external APIs
  or online lookups. The reader loads the file into memory (a country database
  is a few MB); a memory-mapped reader would require a new `unsafe` block
  outside the ADR-045 allowlist, so it is deliberately deferred.
- **No third precedence model.** Country entries live in `allow`, `deny` or
  `rules`; the IP/CIDR caps stay on IP/CIDR entries.
