# WAF Challenge (real protocol)

```text
WAF_CHALLENGE_REAL = YES
WAF_CHALLENGE_STATIC_STUB_ALLOWED = NO
```

Authority: `crates/exyonq-waf/src/challenge.rs`, host wiring in `core/src/waf.rs`.

Historical “static HTML 403 stub” and “interactive challenge deferred” package notes
are **obsolete**.

## Protocol summary

| Field | Implementation |
| --- | --- |
| Protocol | HMAC-SHA256 authenticated challenge + browser PoW + grant cookie |
| Crypto crates | `hmac` 0.13, `sha2` 0.11 |
| Key | 32 bytes from OsRng, or `EXYONQ_WAF_CHALLENGE_KEY_FILE` (exactly 32 raw bytes) |
| Challenge TTL | 60 seconds |
| Grant TTL | 300 seconds |
| PoW difficulty | 16 leading zero bits (SHA-256 of `challenge_id:nonce`) |
| Replay | Bounded used-challenge-id set (capacity **4096**) + grant HMAC/expiry |
| Verify path | Exact `POST /.exyonq/waf-challenge` |
| Cookie | `exyonq_waf_grant` — `HttpOnly; SameSite=Lax; Path=/; Max-Age=300` |

## Issuance

When `[waf.abuse] mode = "challenge"` and the client exceeds the token bucket,
the host returns HTTP **403** with an HTML page containing **inline** JS that:

1. Reads challenge id, HMAC, expiry, difficulty, and return path from the page.
2. Searches for a nonce satisfying the PoW.
3. `POST`s the proof to `/.exyonq/waf-challenge`.

Responses include `Cache-Control: no-store`. No external CDN / no `eval` / no
third-party scripts.

## Binding

Challenge MAC and grant MAC bind:

- challenge id / grant fields
- trusted client IP
- host
- expiry
- difficulty
- generation (embedded in grant token)

Do not trust client-supplied forwarding headers for binding; use ExyonQ’s
resolved client identity.

## Verification

1. Signature WAF still runs on the verify path (Block still applies).
2. Abuse is **skipped** on the verify path (avoids recursive Challenge).
3. Server checks HMAC (constant-time compare), expiry, PoW, and replay set.
4. Success: set grant cookie; JSON `{"ok":true}` or `302` to a same-origin
   relative `return_to` path.

## Generation semantics

```text
CHALLENGE_GENERATION_SEMANTICS:
  grant embeds generation and MACs it
  grant remains valid across N → N+1 until expiry while signing key unchanged
  key rotation (new OsRng key or new key file) invalidates outstanding grants
  valid grant never bypasses independent signature Block
```

## Key security

- No hardcoded signing key in source or docs.
- No key material in configuration examples.
- Key bytes are never logged.
- Use `EXYONQ_WAF_CHALLENGE_KEY_FILE` for stable keys across restarts when required.
