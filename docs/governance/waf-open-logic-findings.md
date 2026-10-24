# WAF open logic findings

```text
DOCUMENT = docs/governance/waf-open-logic-findings.md
TRACK = CAPABILITY_015
STATUS = RECONCILED_DURING_CAP015_MATRIX
```

Owner Cap015 policy: P3 severity is not permission to leave reachable product
defects open. P3-A/P3-B were fixed at Cap015 start; P3-C/P3-D were found and
fixed during Cap015 real E2E before dual-Linux terminal close.

## WAF-LOGIC-P3-B — proxy body error → empty body (FIXED)

```text
FINDING_ID = WAF-LOGIC-P3-B
FINAL_CLASSIFICATION = REAL_PRODUCT_SECURITY_DEFECT
STATUS = FIXED
PRODUCT_PATH_EXISTS = YES
PRODUCTION_REACHABLE = YES
BODY_READ_ERROR_REALISTIC = YES
WAF_RECEIVES_EMPTY_BODY_AFTER_ERROR = NO (after fix)
BACKEND_CAN_CONTINUE = NO (after fix)
SECURITY_INSPECTION_CAN_BE_BYPASSED = NO (after fix)
FAIL_POLICY_BYPASSED = NO (after fix)
MUST_FIX_NOW = YES (done)
```

**Pre-fix path:** `ProxyRequestBodyReadError::Body` mapped to `Some(Vec::new())`,
then WAF body inspect on empty bytes and upstream dispatch continued.

**Fix:** Body-frame transport errors return `BadGateway` without WAF empty-body
inspection and without upstream dispatch (`core/src/server/handler.rs`).

## WAF-LOGIC-P3-A — multi-store Cap013 commit window (FIXED)

```text
FINDING_ID = WAF-LOGIC-P3-A
FINAL_CLASSIFICATION = REAL_PRODUCT_ATOMICITY_SECURITY_DEFECT
STATUS = FIXED
PRODUCT_PATH_EXISTS = YES
PRODUCTION_REACHABLE = YES
ATOMICITY_CAN_BREAK = NO (after fix)
WAF_RUNTIMEPLAN_MISMATCH_POSSIBLE = NO (after fix)
SECURITY_DECISION_STALE_WINDOW_POSSIBLE = NO (after fix)
MUST_FIX_NOW = YES (done)
```

**Pre-fix path:** Shared `NativeWafEngine` ArcSwap meant `commit_prepared` mutated
WAF snapshots for in-flight `ServerState` holders before/while SharedServerState
swap published the matching generation. Abuse/enforce were separate global stores.

**Fix:** Generation-frozen `FixedWafEngine` + `WafRuntimeBinding` (engine + abuse +
enforce) captured into each `ServerState`. Cap013 prepare stages a candidate
binding; new state clones it; commit promotes binding to current without mutating
prior generations' frozen engines.

## WAF-LOGIC-P3-C — proxy wire GET path skipped WAF (FIXED)

```text
FINDING_ID = WAF-LOGIC-P3-C
FINAL_CLASSIFICATION = REAL_PRODUCT_SECURITY_DEFECT
STATUS = FIXED
PRODUCT_PATH_EXISTS = YES
PRODUCTION_REACHABLE = YES
SECURITY_INSPECTION_CAN_BE_BYPASSED = NO (after fix)
MUST_FIX_NOW = YES (done)
```

**Pre-fix path:** HTTP/1.1 `GET /api…` took the proxy wire fast path
(`serve_proxy_wire`) without header-phase WAF. Hyper/POST paths enforced; wire
GET could reach upstream with signature-matching query/headers.

**Fix:** `serve_proxy_wire` runs `inspect_request_headers` before upstream wire
forward and writes the canonical WAF reject on the socket when blocking
(`core/src/server/wire_dispatch.rs`).

## WAF-LOGIC-P3-D — Cap013 ServerState captured WAF before prepare (FIXED)

```text
FINDING_ID = WAF-LOGIC-P3-D
FINAL_CLASSIFICATION = REAL_PRODUCT_ATOMICITY_SECURITY_DEFECT
STATUS = FIXED
PRODUCT_PATH_EXISTS = YES
PRODUCTION_REACHABLE = YES
MIXED_RUNTIME_WAF_GENERATION_OBSERVED = NO (after fix)
MUST_FIX_NOW = YES (done)
```

**Pre-fix path:** `reload_from_path` built `ServerState` (freezing
`waf`/`waf_enforce`/`waf_abuse`) *before* `prepare_waf_reload` staged the
candidate binding. Commit updated process current binding, but the published
generation kept the previous generation's frozen WAF — so reloads of mode,
exclusion, and abuse did not affect new requests.

**Fix:** Stage WAF prepare before `ServerState::new_with_generation_unpublished`
so the new generation freezes the candidate binding (`core/src/reload/mod.rs`).

## WAF-LOGIC-P1-E — static wire / epoll skipped WAF (FIXED)

```text
FINDING_ID = WAF-LOGIC-P1-E
FINAL_CLASSIFICATION = REAL_PRODUCT_SECURITY_DEFECT
STATUS = FIXED
PRODUCT_PATH_EXISTS = YES
PRODUCTION_REACHABLE = YES
SECURITY_INSPECTION_CAN_BE_BYPASSED = NO (after fix)
MUST_FIX_NOW = YES (done)
```

**Pre-fix path:** `WirePlan::Static` and Linux sync/epoll static pumps served
`might_use_static_wire` heads without header-phase WAF. Cap015 Hyper `/site/?…`
tests missed wire-eligible prefixes such as `/site/routes/route*`.

**Fix:** Evaluate generation-frozen WAF before `serve_static_tcp` and before sync
static/epoll bench writes; when WAF is enabled/enforcing, epoll admit forces Hyper
instead of StayInMap (`wire_dispatch.rs`, `connection_executor.rs`).

## WAF-LOGIC-P2-F — proxy wire UTF-8 header erase (FIXED)

```text
FINDING_ID = WAF-LOGIC-P2-F
FINAL_CLASSIFICATION = REAL_PRODUCT_SECURITY_DEFECT
STATUS = FIXED
```

**Pre-fix:** `parse_wire_header_pairs` required the entire head to be UTF-8;
one bad header byte emptied the WAF header view.

**Fix:** Line-oriented UTF-8 parse; skip only invalid lines.

## WAF-LOGIC-P2-G — non-UTF8 header values skipped before WAF (FIXED)

```text
FINDING_ID = WAF-LOGIC-P2-G
FINAL_CLASSIFICATION = REAL_PRODUCT_SECURITY_DEFECT
STATUS = FIXED
PRODUCT_PATH_EXISTS = YES
PRODUCTION_REACHABLE = YES
SECURITY_INSPECTION_CAN_BE_BYPASSED = NO (after fix)
MUST_FIX_NOW = YES (done)
```

**Pre-fix path:** Hyper `header_pairs_from_map` used `HeaderValue::to_str()` and
dropped non-UTF8 values; detector `scan_request_fields` / `check_bot` also skipped
invalid UTF-8. A header like `X-Evil: <script>\xff` could bypass header-phase
signature inspection.

**Fix:** Lossy UTF-8 decode for Hyper pairs, wire header lines, and detector
header/UA scanning (same contract as body inspection).

## WAF-LOGIC-P1-H — non-UTF8 request-target skipped wire WAF (FIXED)

```text
FINDING_ID = WAF-LOGIC-P1-H
FINAL_CLASSIFICATION = REAL_PRODUCT_SECURITY_DEFECT
STATUS = FIXED
PRODUCT_PATH_EXISTS = YES
PRODUCTION_REACHABLE = YES
SECURITY_INSPECTION_CAN_BE_BYPASSED = NO (after fix)
MUST_FIX_NOW = YES (done)
```

**Pre-fix path:** `evaluate_wire_waf` used strict `from_utf8` on the request-target;
failure returned `WafHookResult::Continue`, so static/proxy wire could serve without
signature/abuse inspection when the target contained invalid UTF-8.

**Fix:** Lossy UTF-8 decode of the wire request-target (same contract as P2-G headers);
missing target still inspects headers rather than skipping WAF.

```text
OPEN_WAF_LOGIC_FINDINGS = 0
```
