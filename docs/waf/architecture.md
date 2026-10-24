# WAF architecture

## Dependency boundary

| Edge | Status |
|------|--------|
| `exyonq-core` → `exyonq-waf-api` | required |
| `exyonq-core` → `exyonq-waf` | **forbidden** |
| CLI (`exyonq`) → `exyonq-waf` + `exyonq-waf-api` + `exyonq-ratelimit` | required |

Validate:

```bash
cargo tree -p exyonq-core -i exyonq-waf      # must not resolve
cargo tree -p exyonq-core -i exyonq-waf-api  # must resolve
```

## Request security order (final invariant)

WAF runs on the **client-visible** request URI **after** ExyonQ has resolved the
trusted client identity for the connection, and **before** htaccess / structural
rewrite, FPC/cache hit, static file serve, proxy, FastCGI, and WebSocket upgrade
forwarding.

```text
trusted client identity
  → probe / ACME short-circuit (narrow; see operations.md)
  → WAF header inspect (+ abuse when applicable)
  → challenge verify path (signature WAF yes; abuse skipped)
  → rewrite / route execution
  → FPC / static / proxy / FastCGI / WebSocket
  → after bounded body collect: WAF body inspect (no abuse)
```

Invariants:

- Cache/FPC must not return application content that skipped WAF on an equivalent
  client-visible request.
- Original client path is inspected; rewrite must not be the only WAF view of the
  request.
- WebSocket: header WAF before upgrade forward.
- HTTP/1.1, HTTP/2, and HTTP/3 share the same WAF security hooks on their
  respective serve paths.

```text
WAF_CLIENT_IP_SOURCE = EXYONQ_CANONICAL_TRUSTED_CLIENT_IDENTITY
```

The engine consumes the host-resolved identity already attached to the connection
context. It does **not** independently trust arbitrary client `X-Forwarded-For`,
`Forwarded`, or `X-Real-IP` headers.

## Decision precedence

Ordered from strongest security outcome:

1. **Signature inspect** produces `Block` while enforce is on → host reject
   (backend / FastCGI / WebSocket upgrade do not proceed).
2. Else **abuse** (unless a valid challenge grant cookie allows the client past
   the abuse gate) → `Challenge` or `RateLimit`.
3. In **monitor** mode, signature `Block` is coerced to `Log` (no host reject for
   signatures). Abuse Challenge/429 still apply if configured.
4. **Exclusion** suppresses matching signature rule IDs only; it does not disable
   the WAF engine or erase unrelated Blocks.
5. Weaker outcomes (`Allow` / `Log`) never erase a stronger signature `Block`
   already decided for the request.

```text
CHALLENGE_VERIFICATION_DOES_NOT_BYPASS_SIGNATURE_BLOCK = YES
RATE_LIMIT_DOES_NOT_ERASE_SIGNATURE_BLOCK = YES
```

## Protocol / backend wiring

| Path | Wired through WAF security path |
|------|----------------------------------|
| HTTP/1.1 | Yes |
| HTTP/2 | Yes (shared handler path) |
| HTTP/3 | Yes (shared semantic hooks) |
| Static / `/site/*` | Yes (before static serve) |
| Proxy | Yes (headers before; body after bounded collect) |
| FastCGI | Yes |
| WebSocket | Yes (pre-upgrade headers) |

```text
WIRED ≠ CAPABILITY_015_VERIFIED_REAL_PRODUCTION
```

## Reload / generation

WAF participates in Cap013 prepare/commit — not an independent async reload.

1. Reload prepare compiles a candidate **generation-frozen**
   `WafRuntimeBinding` (FixedWafEngine + abuse + enforce) from candidate
   `AppConfig` **before** generation publish.
2. Prepare failure → discard candidate; live generation **N** unchanged.
3. New `ServerState` captures that frozen binding; commit promotes it to current
   together with SharedServerState publish.
4. Identical config fingerprint remains a reload **NO-OP**.

In-flight requests keep their `ServerState`’s frozen WAF binding. A newly
admitted request must not observe a RuntimePlan generation paired with a WAF
binding that was never committed for that generation.

## Response phase (H6)

No response-body/header detectors are implemented. There is no productive
response-phase WAF hook in the current product contract.
