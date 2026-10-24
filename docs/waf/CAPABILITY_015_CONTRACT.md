# Capability 015 — WAF acceptance contract

```text
CAPABILITY_ID = 015
CAPABILITY_NAME = waf
CAP015_PRIMARY_REALITY_STATUS_ORIGINAL = DOC_ONLY
CAP015_PRE_QUALIFICATION_IMPLEMENTATION_STATUS = IMPLEMENTED_NOT_YET_VERIFIED_REAL_PRODUCTION
CAP015_OWNER_CLASSIFICATION = REQUIRED_CURRENT_PRODUCT_CAPABILITY
TERMINAL_STANDARD = VERIFIED_REAL_PRODUCTION
```

Derived from integrated product code and `docs/waf/**` (not the retired staging package).

## CAPABILITY_015_PRODUCT_CONTRACT

Native ExyonQ WAF on `exyonq serve`: signature engine (`monitor` / `block` / `disabled`)
plus singleton abuse (`off` / `rate_limit` / `challenge`), full IR
(`[waf]` / `[waf.abuse]` / `[[waf.ruleset]]` / `[[waf.exclusion]]`), Cap013
generation-frozen bindings, real HMAC-PoW challenge, real SyncTokenBucket rate
limit, hooks before FPC/static/proxy/FastCGI/WebSocket on HTTP/1.1+H2+H3 paths.

## CAPABILITY_015_SUPPORTED_BEHAVIOR

- Modes: disabled / monitor / block with documented host-enforce semantics
- Built-in EXY-* detector families + custom ruleset rules
- Exclusions (host / path_prefix / method / rule_ids) scoped only
- Body inspection budget + `on_inspection_limit`
- Abuse rate_limit (429 + Retry-After) and real Challenge protocol
- Cap013 prepare/commit/discard; invalid reload keeps last
- Trusted client identity from connection peer (not untrusted client XFF)
- Fail-closed proxy request-body transport errors (502; no empty-body WAF)

## CAPABILITY_015_EXPLICIT_NON_SCOPE

- ModSecurity CRS import
- ML / anomaly scoring
- Response-phase (H6) detectors
- CAPTCHA SaaS / external challenge provider
- Competitive RPS / official bench
- Capability 016+
- Treating pre-Cap015 dual-Linux probes as sole terminal proof

## CAPABILITY_015_DEPENDENCIES

- Cap013 atomic reload semantics
- Cap004–012/014 path invariants when those surfaces are traversed
- Post-WAF dependency seal (`POST_WAF_CARGO_LOCK_SHA256`)

## CAPABILITY_015_CURRENT_EVIDENCE

- Integration probes (supplemental): Netcup/Oracle benign 200 / XSS 403
- Cap015 REAL E2E dual-Linux harness: `scripts/reality/cap-015-waf-e2e.py`

## CAPABILITY_015_KNOWN_LIMITATIONS

- No response-phase detectors
- Abuse cardinality singleton only
- Challenge difficulty default fixed at engine default (16)

## CAPABILITY_015_KNOWN_OPEN_LOGIC_NOTES

```text
OPEN_WAF_LOGIC_FINDINGS = 0
```

Former P3-A/P3-B fixed before Cap015 matrix execution
(`docs/governance/waf-open-logic-findings.md`).
