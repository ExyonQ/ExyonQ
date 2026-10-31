# WAF configuration

Authority: `config/ir/src/waf.rs` (validated at parse; fingerprints with `AppConfig`).

```text
WAF_INVALID_CONFIG_BEHAVIOR = FAIL_CLOSED
WAF_CONFIG_CHANGE_FINGERPRINTS = YES
IDENTICAL_WAF_CONFIG_RELOAD_NO_OP = YES
```

Malformed WAF IR fails config load. Failed WAF compile at serve/reload **prepare**
aborts startup/reload; the previous generation remains active.

## `[waf]` root

| Field | Type | Default | Allowed | Runtime effect |
| --- | --- | --- | --- | --- |
| `enabled` | bool | `true` | bool | `false` forces detectors off / no-op compile path |
| `mode` | string | `"monitor"` | `monitor` \| `block` \| `disabled` | Signature enforcement mode (see PRODUCT.md) |
| `engine` | string | `"native"` | `native` only | Other values rejected |
| `max_inspection_body_bytes` | u64 | `1048576` | `>= 1` | Body inspection budget |
| `on_inspection_limit` | string | `"log_and_allow"` | `allow_uninspected` \| `block` \| `log_and_allow` | Behavior when budget exhausted |
| `fail_policy` | string | `"closed_for_invalid_rules"` | that value only | Compile/config fail-closed |

### `on_inspection_limit`

| Value | Effect |
| --- | --- |
| `allow_uninspected` | Continue without treating truncation as Block |
| `block` | Emit `EXY-LIMIT-BODY-001` as Block when enforce applies |
| `log_and_allow` | Emit limit evidence / Log; request may proceed |

## `[waf.abuse]` — singleton

```text
WAF_ABUSE_CARDINALITY = SINGLETON
```

Use a **table**, not an array:

```toml
[waf.abuse]
mode = "off"                 # off | rate_limit | challenge
requests_per_second = 10
burst = 20
```

| Field | Type | Default | Validation | Runtime effect |
| --- | --- | --- | --- | --- |
| `mode` | string | `"off"` | `off` \| `rate_limit` \| `challenge` | Abuse gate mode |
| `requests_per_second` | u32 | `10` | `>= 1` when mode ≠ `off` | Token bucket refill |
| `burst` | u32 | `20` | `>= 1` when mode ≠ `off` | Token bucket capacity |

Rationale: one process-global IP-keyed policy. Multiple overlapping abuse policies
would have undefined same-IP precedence.

Implementation: composition root `SyncTokenBucket` (`modules/ratelimit`).
Rate-limit response: HTTP **429** with **`Retry-After`** from
`SyncTokenBucket::retry_after_secs` (minimum 1 second).

## `[[waf.ruleset]]`

Repeatable. Duplicate `id` → config reject.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `id` | string | required | Non-empty; unique across rulesets |
| `enabled` | bool | `true` | Disabled ruleset rules are skipped |
| `builtins` | table (optional) | — | Optional detector toggles (see rules.md) |
| `rule` / `[[waf.ruleset.rule]]` | array | empty | Custom signature rules |

When WAF is enabled and no ruleset supplies `builtins`, the engine uses the
**default built-in detector pack** (SQL/XSS/path/cmd Block; header/bot Log).

Custom rule fields:

| Field | Type | Validation |
| --- | --- | --- |
| `id` | string | Non-empty; unique across all rulesets |
| `enabled` | bool | default `true` |
| `pattern` | string | Non-empty regex |
| `target` | string | `path` \| `query` \| `body` \| `header:<name>` |
| `action` | string | `allow` \| `log` \| `block` (not challenge/rate_limit) |
| `phases` | string[] | At least one of connection / request_headers / request_body_* / response_* |

## `[[waf.exclusion]]`

Repeatable. Scoped signature suppression only.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `host` | string? | none | Exact host match when set |
| `path_prefix` | string? | none | Exact path or `prefix + "/" + …` (not `/api` vs `/api2`) |
| `method` | string? | none | Method match when set |
| `rule_ids` | string[] | required non-empty | Allowlist of rule IDs; empty rejected at config |

Malformed exclusions fail config parse. Empty `rule_ids` never means “all rules”.

## Example

```toml
[waf]
enabled = true
mode = "block"
engine = "native"
max_inspection_body_bytes = 1048576
on_inspection_limit = "log_and_allow"
fail_policy = "closed_for_invalid_rules"

[waf.abuse]
mode = "rate_limit"
requests_per_second = 10
burst = 20

[[waf.ruleset]]
id = "exyonq-core"
enabled = true

[[waf.exclusion]]
host = "example.com"
path_prefix = "/upload"
rule_ids = ["EXY-XSS-1001"]
```

## Environment overrides

When set (non-empty), env overrides IR at the composition root:

| Env | Effect |
| --- | --- |
| `EXYONQ_WAF_MODE` | `monitor` \| `block` \| `disabled` |
| `EXYONQ_WAF_ENFORCE` | Host reject on signature Block (`1`/`true`/…); ignored unless mode is `block` |
| `EXYONQ_WAF_ABUSE` | `off` \| `rate_limit` \| `challenge` (**unknown → fail closed**) |
| `EXYONQ_WAF_ABUSE_RPS` / `EXYONQ_WAF_ABUSE_BURST` | Numeric overrides |
| `EXYONQ_WAF_CHALLENGE_KEY_FILE` | Path to exactly 32 raw key bytes |

```text
PRECEDENCE = ENV overrides IR when set
CANONICAL_POLICY_OWNER = IR ([waf] / [waf.abuse]) when env unset
```

## Reload

WAF fields that affect runtime participate in the canonical config fingerprint.
Cap013 prepare compiles the candidate WAF+abuse configuration before commit.
