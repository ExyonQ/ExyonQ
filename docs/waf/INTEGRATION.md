# WAF local verify checklist

Authoritative product docs: [README.md](./README.md).

This checklist is for developers validating the integrated product locally.
It is **not** Capability 015 production qualification.

## Dependency boundary

```bash
cargo tree -p exyonq-core -i exyonq-waf      # must not resolve as a core dependency
cargo tree -p exyonq-core -i exyonq-waf-api  # must exist
```

## Tests

```bash
cargo test -p exyonq-waf-api -p exyonq-waf
cargo test -p exyonq-config-ir waf::
cargo test -p exyonq-core --lib waf::
```

## Serve examples

Monitor (signature Block → Log; no host reject for signatures):

```toml
[waf]
mode = "monitor"
```

```bash
exyonq serve ./config.toml
```

Block + rate limit:

```toml
[waf]
mode = "block"

[waf.abuse]
mode = "rate_limit"
requests_per_second = 10
burst = 20
```

```bash
EXYONQ_WAF_ENFORCE=1 exyonq serve ./config.toml
```

## Cap013

Reload prepare compiles the candidate WAF snapshot and abuse gate before publish.
Failure discards the candidate; live WAF stays on generation N.
See [architecture.md](./architecture.md).
