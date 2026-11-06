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
| Providers | `native` (default), reCAPTCHA v2 checkbox, hCaptcha; reCAPTCHA v3 is not implemented |
| Crypto crates | `hmac` 0.13, `sha2` 0.11 |
| Key | 32 bytes from OsRng, or `EXYONQ_WAF_CHALLENGE_KEY_FILE` (exactly 32 raw bytes) |
| Challenge TTL | 60 seconds |
| Grant TTL | 300 seconds |
| PoW difficulty | 16 leading zero bits (SHA-256 of `challenge_id:nonce`) |
| Replay | Bounded used-challenge-id set (capacity **4096**) + grant HMAC/expiry |
| Verify path | Exact `POST /.exyonq/waf-challenge` |
| Cookie | `exyonq_waf_grant` — `HttpOnly; SameSite=Lax; Path=/; Max-Age=300` |

`[waf.abuse] mode = "challenge"` is independent of the signature `waf.mode`.
It therefore issues a Challenge even when signatures are in `monitor` mode; a valid
grant never bypasses an independent signature Block.

## Issuance

When the client exceeds the token bucket, the host returns HTTP **403** with an HTML
page that:

1. Reads challenge id, HMAC, expiry, difficulty, verify path and return path.
2. Searches for a nonce satisfying the PoW.
3. `POST`s the proof to `/.exyonq/waf-challenge`.

The native page uses **inline JS only** — no external CDN, no `eval`, no third-party
scripts. With `provider = "recaptcha"` or `"hcaptcha"`, the same page loads the
provider’s official widget script, waits for the widget token (bounded poll), and
still performs the native PoW. A provider token is never accepted by itself.

Responses include `Cache-Control: no-store`.

## No-JS fallback

The challenge page contains an explicit `<noscript>` block stating that JavaScript
is required, with instructions to enable it and reload. It does **not** grant access,
set a cookie, redirect, or expose an alternate permissive path. A client with JS
disabled remains at the 403 challenge until it can solve the configured challenge.
This is a fail-closed fallback, not an access mechanism.

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
3. Server checks HMAC (constant-time compare), expiry, PoW, and replay set first.
4. For an external provider, the server then verifies `provider_token` against the
   configured `verify_url` with a bounded HTTPS `POST` (5 s, 16 KiB response).
5. A missing/empty secret, provider rejection, timeout, non-200 response or malformed
   JSON is fail-closed (`403`); no grant cookie is issued.
6. Success: set grant cookie; JSON `{"ok":true}` or `302` to a same-origin
   relative `return_to` path.

Provider secrets are read from the environment variable named by `secret_env` at
verification time and are never serialized, logged or returned. The provider client
uses OS trust roots and fails closed if they are unavailable.

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

## Measured verification (2026-11-04)

Criterion, literal from `Roadmap 0.9.1` (OL-04, line 56):

> *Criterio de aceptación:* Challenge real funcionando; configuración por despliegue; fallback sin JS

| Criterion surface | Result | Evidence |
| --- | --- | --- |
| Challenge real funcionando | PASS | native 16/16 and `monitor` 16/16 over real HTTP: PoW solved by the client, HMAC challenge, grant cookie (HttpOnly/SameSite=Lax/Max-Age=300), forged MAC 403, replay 403, tampered grant 403 |
| Configuración por despliegue | PASS | `[waf.abuse]` + `EXYONQ_WAF_CHALLENGE_KEY_FILE` (exactly 32 bytes); five configs validated; a provider without `site_key`/`secret_env` is rejected by `exyonq validate` (exit 1) |
| Fallback sin JS | PASS | every page carries `<noscript>`; the native page has no external `<script src>`; a client without JS stays at 403 with no cookie and no redirect |
| Native Linux amd64 | PASS | Netcup, `exyonq` `7052c6564a4597d64c52a0db7458343561fe5e4084a6932bb82853920de3a883`: 83/83, `RESULT=PASS OL-04`, exit 0 (`netcup-native-20261104T233527Z.log`, SHA-256 `6b3a26248c059f50f5d53bccfa8d4c782112f50d644be01176a6d76edf0e32ab`) |
| Native Linux arm64 | PASS | Oracle, `exyonq` `2fd2085c0c0ef333a667098a8cb0186577d2f93eed3856e329603f6866e296a5`: 83/83, `RESULT=PASS OL-04`, exit 0 (`oracle-native-20261104T233532Z.log`, SHA-256 `2a3b0e7e990d37ad16636e4a307162959591da1cb6bcc3864e34d8e560b410f1`) |
| External providers | PASS | reCAPTCHA v2 19/19 and hCaptcha 19/19 against the real `siteverify` endpoints with the vendors' official test keys; invalid secret 13/13 fails closed 403 with no grant |

### Runs

Darwin arm64, `target/debug/exyonq` SHA-256
`c5769c7e832f0243e02668539b5e845ab7da73e423d47634c755aaed86d480c1`
(`source_revision=fb33e81c…`), harness SHA-256
`6f4b3117c7e712b575b3717aad0fc4000bc955cd55c923db62e4a30a41610a47`:

```bash
bash scripts/e2e/ol04-challenge-e2e.sh
```

native 16/16, monitor 16/16, reCAPTCHA 19/19, hCaptcha 19/19, invalid-secret
13/13 (83 checks), `pass=11 fail=0`, `RESULT=PASS OL-04`. Log
`.exyonq-local/evidence/ol04/full-20261104T165031Z.log`, SHA-256
`58d95f5272da314c403cd58d0c0a08fc24ce49760ddb210d83bae386f7a4cf2e`.

Linux arm64 (container `exyonq-test:wolfi`, `Linux 7.0.14-linuxkit aarch64`),
`exyonq` SHA-256
`0cc88c562eadb99e5be2a88fd67155446b0308a170843e4084a9a34221e7f158`, same
harness and assertions: native 16/16, monitor 16/16, reCAPTCHA 19/19, hCaptcha
19/19, invalid-secret 13/13, `RESULT=PASS OL-04`. Log
`.exyonq-local/evidence/ol04/linux-full-20261104T165057Z.log`, SHA-256
`441ca5d458b9e94f9d9ad5944197219fa18e1db8334a70dd605a2edd7b730f95`.

Provider connectivity was verified independently before the runs: a real
`POST` to `https://www.google.com/recaptcha/api/siteverify` and to
`https://hcaptcha.com/siteverify` with the official test keys returned HTTP 200
and `"success": true`.

### Harness finding (fixed)

The first run (`full-20261104T164759Z.log`, SHA-256
`fc3d089956a97056c02fc66e84e81c382647e81cc349548d9ca8ff80fb10ae2a`) failed one
assertion in each provider phase: `tampered grant rejected` returned 200 instead
of 403. The product order is grant check → quota gate; the abuse quota is a
one-token bucket and the provider round trip took the second that separated the
two requests, so the forged-grant request passed **on quota**, not on the grant.
The harness now drains the bucket immediately before the assertion (no assertion
relaxed), and `forged_or_mismatched_grant_is_rejected` was added to
`crates/exyonq-waf/src/challenge.rs` (tampered MAC, MAC minted for another host,
MAC minted for another client IP, expired grant). Both are green.

### Complementary tests

`cargo test -p exyonq-waf`: 78 unit (77 before this increment) + 9 golden;
`cargo test -p exyonq-core --lib waf::tests`: 8/8;
`cargo test -p exyonq-core --lib captcha::tests`: 2/2;
`cargo clippy -p exyonq-waf -p exyonq-core -p exyonq --all-targets --locked -- -D warnings`
and `cargo fmt --all -- --check`: PASS.

### OpenLiteSpeed differentials (1.9.4 source)

Not claimed as parity, recorded against `src/main/httpserver.cpp:4658`
(`initLsrecaptcha`) and `dist/docs/ServSecurity_Help.html`:

- OpenLiteSpeed activates CAPTCHA on a connection limit, anti-DDoS suspicion or a
  rewrite-rule environment variable; ExyonQ activates it on the `[waf.abuse]`
  quota gate only.
- OpenLiteSpeed's default `recaptchaType` is *reCAPTCHA Invisible*; ExyonQ renders
  the v2 checkbox widget (and hCaptcha) and always runs the native PoW.
- OpenLiteSpeed exposes `recaptchaMaxTries`, `recaptchaAllowedRobotHits` and a
  bot whitelist; ExyonQ does not implement those knobs.

### Limits

Linux was executed natively on 2026-11-04, over SSH with the binary built on
each host: Netcup amd64 (`7052c656…`, log `6b3a2624…`) and Oracle arm64
(`2fd2085c…`, log `2a3b0e7e…`), 83/83 and `RESULT=PASS OL-04` on both. The
earlier Wolfi container run remains historical evidence, not a substitute for
these native runs. Providers were exercised with official test keys, not
production accounts. reCAPTCHA v3 is not implemented or claimed; `maxTries` and
whitelist are declared OLS differentials, not part of the 0.9.1 criterion.

### Audit stack (2026-11-04, §8)

Run on this tree after the E2E and tests above. The docs edit that adds this
section came after the CodeGraph index and changes no Rust source.

- **Leg 1 — Veyra:** `veyra audit . --profile standard` → decision **ACCEPTED**,
  `VEYRA_EXIT=0`, evidence `.veyra/evidence/20261104T163957Z/report.md` (plus
  `report.json` and `manifest.json`). OK: rustfmt, cargo check, clippy, cargo
  test, cargo-audit, veyra-anti-theater, nextest, llvm-cov, cargo-deny,
  cargo-machete. **INCONCLUSIVE:** cargo-geiger (coverage warnings in 54 runs;
  unsafe counts are a lower bound — not a pass). **SKIPPED:** dylint (no
  configuration), cargo-semver-checks (unpublished crates have no baseline),
  veyra-e2e (no `[[e2e.scenarios]]`). No blocking findings.
- **Leg 2 — Rust Audit:** rust-ai-audit 0.2.7,
  `scan . --format md --out report --fail-on high` → exit **1**,
  `report/rust-audit-20261104-180101.md`: 31 critical, 1586 high, 1133 medium,
  459 low, 7 info (2026-11-03 baseline: 35/1666/1120/448/7 — no new critical,
  lower totals). The 17 findings in files touched by OL-03/OL-04 were reviewed
  and none is a real defect: the `hmac_hex` `expect` has a stated invariant
  (HMAC-SHA256 accepts any key length; key is `[u8;32]`), the
  `core/src/waf.rs` `expect` / `let _ =` / `parse_client_ip` hits are
  preexisting lines outside this increment, `captcha::url_decode`'s `.ok()`
  keeps a literal `%xx` on undecodable input, and an absent provider token
  defaults to `""` so the provider check returns 403 (fail-closed). Remaining
  criticals are preexisting (including the deliberate
  `scripts/gates/fixtures/must-fail/` fixture) and were not re-triaged here.
- **Leg 3 — CodeGraph:** two stores, one writer each.
  - *Full structural graph* (`index . --index-tier fast --force`, embeddings and
    semantic resolution off) → exit 0: 637 files, 204 382 lines, 24 155 nodes,
    28 916 edges, `graph_complete=true`, `vector_index_ready=false`,
    `errors=0`, `index_ms=457263`.
  - *Cycles* over that graph: `agent impact --focus dependencies` with
    `CODEGRAPH_AGENT_RESULT_BUDGET_BYTES=500000` → exit 0. Verdict: **no
    circular dependencies** at crate or module granularity (`Imports`, `Uses`,
    `Extends`, `Implements` all empty); the only `Calls` pairs are intra-file
    mutual recursion (`compat/nginx/src/parser.rs:71` ↔ `:113`, bounded by
    `MAX_BLOCK_DEPTH`) and name-resolution false positives. Evidence
    `.exyonq-local/evidence/ol04/cg-cycles.json` (SHA-256 `c54ceff4…`).
  - *Vector index* for `agent context`: the full graph's `chunks` HNSW index was
    unusable after the stopped embedding run (`Vector index 4 on table chunks
    cannot be searched: entry point element 16776 has no vector`), so a second,
    scoped store was built with `--include '**/crates/exyonq-waf/**'` plus the
    other OL-03/OL-04 paths: 19 files, 11 014 lines, 1 154 nodes, 1 154
    embeddings, `vector_index_ready=true`, `errors=0`, `index_ms=86502`
    (`cg-index-vector2.json`, SHA-256 `cef1be82…`). The full-graph store is kept
    as `.codegraph/db.full-<timestamp>`.
  - *Context*: `agent context "Where is the WAF challenge grant issued and
    verified?" --focus search` → exit 0 with real `file:line`:
    `crates/exyonq-waf/src/challenge.rs:146/208/289/333/389/411`,
    `core/src/waf.rs:180/649`, `crates/exyonq-waf-api/src/lib.rs:268`
    (`cg-context-vector2.json`, SHA-256 `488ae907…`). The throttle query returns
    `core/src/client_throttle.rs:95/134/192/277/295`
    (`cg-context-throttle.json`, SHA-256 `51b85cf3…`).
  - *Limits*: the vector store covers the WAF/throttle paths only (documented
    convergence; relative `--include` patterns matched zero files and were
    replaced by `**/` prefixes); semantic symbol resolution is off; the cycle
    verdict comes from the full structural graph, not the scoped store.
- **Leg 4 — Hindsight:** **NOT EXECUTED** — the `hindsight-exyonq` MCP server is
  not loaded in this client; no other bank was written.
