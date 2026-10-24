# ExyonQ native WAF

```text
WAF_IMPLEMENTED = YES
CAPABILITY_015_VERIFIED_REAL_PRODUCTION = NO
CAP015_WAF_IMPLEMENTATION_STATUS = READY_FOR_REAL_PRODUCTION_QUALIFICATION
CAPABILITY_015_STARTED = NO
```

Native Rust WAF integrated into the ExyonQ product runtime (`exyonq serve`).
No Pingora runtime dependency. Capability 015 dual-Linux production qualification
is a **separate** campaign and has **not** started.

## Canonical authorities

| Authority | Path |
|-----------|------|
| WAF API contracts | `crates/exyonq-waf-api/**` |
| WAF engine | `crates/exyonq-waf/**` |
| Configuration IR | `config/ir/src/waf.rs` (+ merge/fingerprint wiring) |
| Core host integration | `core/src/waf.rs`, request hooks in `core/src/server/handler.rs` |
| Composition root | `cli/exyonq/src/waf_install.rs` |
| Documentation | `docs/waf/**` (this tree) |

There is **no** separate drop-in / staging package outside the canonical crate
paths above. Product source and docs in this repository are the only authorities.

## Documents

| Doc | Content |
|-----|---------|
| [PRODUCT.md](./PRODUCT.md) | Product overview, modes, Cap015 status boundary |
| [architecture.md](./architecture.md) | Dependency boundary, request order, decision precedence |
| [configuration.md](./configuration.md) | TOML schema, defaults, validation, reload/fingerprint |
| [rules.md](./rules.md) | Built-in detectors, rule IDs, custom rules, exclusions |
| [challenge.md](./challenge.md) | Real HMAC-PoW challenge protocol |
| [operations.md](./operations.md) | Env overrides, body limits, bypasses, limitations |
| [INTEGRATION.md](./INTEGRATION.md) | Local verify checklist (not Cap015 proof) |

## Open nonblocking logic findings

Tracked in [`docs/governance/waf-open-logic-findings.md`](../governance/waf-open-logic-findings.md)
(`WAF-LOGIC-P3-A`, `WAF-LOGIC-P3-B`). Not closed.
