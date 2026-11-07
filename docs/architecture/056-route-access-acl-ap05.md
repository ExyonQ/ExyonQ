# ADR-056 — Declarative per-route access policy (AP-05)

**Status:** Accepted
**Date:** 2026-11-02
**Depends on:** config IR validation, compiled runtime route policy, `.htaccess` overlay (ADR-055)
**Contract:** a configured restriction is enforced before content is served; an unrepresentable import fails closed

## Context

AP-05 closes the route-level ACL gap left by ADR-055. Before this decision, the
product only had the global `[[waf.ruleset]] builtins.ip_filter`; nginx
`allow`/`deny`, Apache `Require ip`/`method`, `Order`/`Allow`/`Deny`, and the
`.htaccess` ACL directives were rejected or silently absent. Operators could
believe a route was protected when it was not.

The native policy is deliberately declarative and small. It is not a general
expression language and does not attempt to reproduce every Apache or nginx
access-control edge case.

## Decision

### IR

A route may carry one access policy:

```toml
[[route]]
name = "private"
match = { path = "/private" }

access = {
  allow = ["10.0.0.0/8", "2001:db8::/32"],
  deny = ["10.1.0.0/16"],
  methods = ["GET", "HEAD"],
  schedule = { days = ["mon", "tue"], start = "22:00", end = "02:00" },
}
```

- `allow` and `deny` accept IPv4/IPv6 addresses, CIDRs, or `all`.
- `rules` is an ordered first-match list of `{ effect = "allow"|"deny", source
  = "IP/CIDR/all" }` entries. It is mutually exclusive with `allow`/`deny` and
  is used by the nginx importer to preserve source order.
- Each list is capped at 64 entries. Duplicates and invalid CIDRs are rejected
  during config validation; no CIDR text is parsed on the request path.
- `methods` is an HTTP-method allowlist, capped at 16 entries. Matching is
  case-insensitive; configured duplicates are rejected.
- `schedule.days` accepts short or full English weekday names. An empty list
  means every day. `start` and `end` are `HH:MM` in UTC.
- `start == end` means all day. A window with `end < start` crosses midnight and
  belongs to the configured start day: a `mon 22:00–02:00` window covers Monday
  22:00–23:59 and Tuesday 00:00–01:59, not Monday 00:00–01:59.
- An empty access table, unknown fields, duplicate entries and malformed values
  are configuration errors.

### Evaluation

For a request that has matched the route:

1. Any matching `deny` entry returns `403 Forbidden`.
2. If `allow` is non-empty and the client IP matches none of its entries,
   return `403`.
3. If `methods` is non-empty and the request method is absent, return `403`.
4. If a schedule is present and the current UTC instant is outside it, return
   `403`.
5. Otherwise continue to the normal route pipeline.

`deny` therefore wins over `allow`; a non-empty `allow` list is an allowlist,
not an additive exception list. The checks run before auth, static, redirect,
proxy and FastCGI dispatch, so an ACL denial does not reveal a backend response
or issue an auth challenge.

An **unconditional** deny-all policy (`deny = ["all"]` with no allowlist,
method filter or schedule) is also a complete route terminal. It needs no
`upstream`, `root`, `redirect`, `rewrite` or `fastcgi`; the route returns `403`
and can be used for path-boundary blocks such as `/wp-config.php`. Conditional
policies are not terminals: if they do not reject a request, a real action must
still exist.

The policy is compiled once per runtime snapshot. The same check runs again
after an internal rewrite/rematch and after a `.htaccess` rewrite that changes
the selected route, so a rewritten URI cannot bypass the new route's policy.

### Sources

- **Apache importer:** `Require ip`, `Require method`, `Require all
  granted/denied`, `Require not ip`, and the common `Order`/`Allow`/`Deny`
  forms are emitted as `[[route]] access`. Order combinations that cannot be
  represented exactly are rejected rather than approximated.
- **nginx importer:** server/location `allow` and `deny`, including `all`, are
  emitted as ordered `access.rules`; first-match order is preserved. `satisfy
  any` is not representable and fails closed.
- **`.htaccess` overlay:** `Require ip`, `Require method`, `Require all
  granted/denied` and `Require not ip` compile into a per-directory policy.
  Legacy `Allow`/`Deny`/`Order`/`Satisfy` remain fail-closed in the overlay as
  required by ADR-055; the Apache file importer handles the common vhost forms
  described above.

`Require env`, `Require group`, `Require expr`, `Satisfy any`
and arbitrary directive-order semantics are out of scope. They are rejected or
represented by a strict deny-all policy; they are never silently ignored.

## Consequences

- ACL decisions are deterministic, cheap and shared by HTTP/1.1, HTTP/2 and
  HTTP/3 request paths.
- A route with `access` cannot use the static fast path; it goes through full
  dispatch. This is intentional and bounded by the config validation limits.
- A `403` is the observable contract. There is no redirect to a login page and
  no `WWW-Authenticate` header for an access denial.
- The native policy is richer than either importer's ACL support. Importers
  must report their approximations; they must not claim total parity.
