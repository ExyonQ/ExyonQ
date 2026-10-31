# V044 — Cap034 proxy framing & keep-alive contract review

```text
STATUS = COMPLETE
CASE = C34-C
WIP = V044_CAP034_PROXY_FRAMING_AND_KEEPALIVE_CONTRACT_REVIEW
MODE = READ_ONLY_PROTOCOL_CONTRACT_AND_EXPECTED_VALUE_REVIEW
PRODUCT_MUTATION = NO
ARCHITECTURE_MUTATION = NO
P5_STARTED = NO
P5_TO_P14_FROZEN = YES
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

SOURCE_HEAD = ade997a362e60d1dc6eafc984da454930600845a
SOURCE_TREE = 4ae6ffc7528d0881be23fcac246c345dcb9a1606
PRODUCT_BINARY_SHA256 = e28c0abe18587f2d911d492be64cbec42372383e011417a6c578be52da5ba459

PARENT_PROFILING = v044-p4-proxy-path-causal-profiling.md (CASE P4P-B)
```

---

## 1. Executive answer

```text
IS_CAP034_CHUNKED_PLUS_CLOSE_REQUIRED_FOR_CORRECTNESS = NO (HTTP protocol)
IS_CAP034_CHUNKED_PLUS_CLOSE_REQUIRED_FOR_CURRENT_WIRE_IMPLEMENTATION = YES
CAN_EXYONQ_SAFELY_PRESERVE_CONTENT_LENGTH_AND_CLIENT_KEEPALIVE_WHEN_RESPONSE_SEMANTICS_ALLOW_IT =
  YES_IN_PRINCIPLE — NOT under current wire architecture without product evolution

RECOMMENDED_CONTRACT_OPTION = OPTION_C (general downstream framing policy) — REVIEW ONLY
NEXT_PRODUCT_WIP_PROPOSAL = NONE
NEXT_MEASUREMENT_WIP_PROPOSAL = V044_CAP034_FRAMING_KEEPALIVE_BOUNDED_VALUE_AB
```

Cap034 is **not** an HTTP correctness requirement for a known-length pass-through proxy response.
It **is** the current deliberate product contract for the **proxy wire fast path** (`GET /api/*`):
one-shot admission, no client keep-alive re-entry, upstream hop headers stripped, downstream
always `Transfer-Encoding: chunked` + `Connection: close`, body streamed via Cap030 progressive
chunk writer.

The P4 competitive gap includes **contract asymmetry** (ExyonQ chunked/close vs NGINX/OLS
Content-Length/keep-alive) **plus** separate implementation costs (Hyper/Tokio stack, scheduler
wakeups, allocator churn — ROOT_1A/1F/1G). This review does **not** attribute the full gap to
Cap034 alone.

---

## 2. Cap034 source reconstruction

### CAP034_DEFINITION

Capability embedded in the **proxy wire fast path** — not a standalone crate flag. Contract:

```text
ONE_WIRE_ADMISSION = ONE_CLIENT_REQUEST
HOST_CLUSTER_RANKING = pinned at admission (generation + Host → cluster_id)
DOWNSTREAM_RESPONSE = strip upstream Connection / TE / Content-Length;
                      emit Connection: close + Transfer-Encoding: chunked;
                      stream body via Cap030 progressive TE writer;
                      shutdown write side after response
NO_KEEPALIVE_RE_ENTRY = intentional (helpers disconnected)
```

### CAP034_CALL_SITES

| Location | Role |
| --- | --- |
| `crates/exyonq-mod-proxy/src/wire_conn.rs` | **Primary** — `might_use_proxy_wire`, `encode_proxy_response_head`, `write_streaming_proxy_response_wire`, `serve`, `write_chunk` |
| `crates/exyonq-mod-proxy/src/kernel_hooks.rs` | Registers wire eligibility + `serve` hook |
| `module-api/src/proxy_wire.rs` | Façade: `might_use_proxy_wire`, `serve_proxy_wire` |
| `core/src/server/wire_dispatch.rs` | `resolve_proxy_route` (Cap034 Host ranking), `serve_proxy_wire` dispatch |
| `core/src/kernel/wire.rs` | Wire plan decision includes proxy wire branch |
| `crates/exyonq-mod-static/src/wire_eligibility.rs` | `epoll_keep_alive_eligible` — **explicitly false** for `GET /api/*` |
| `core/src/server/wire_dispatch.rs` | Tests assert `/api/*` not keep-alive eligible |

### CAP034_INVARIANTS

```text
I1  GET /api/* (eligible head) → proxy wire, not Hyper, when cache/WAF/upgrade gates pass
I2  Host/:authority ranking on wire matches Hyper path (no hostless-over-named shadowing)
I3  One request per wire admission; socket write-half shutdown after response
I4  Downstream never forwards upstream Connection / TE / Content-Length verbatim
I5  Downstream always advertises Connection: close + TE: chunked on success path
I6  Body always streamed (Cap030); never full-collected merely because upstream declared CL
I7  Keepalive re-entry helpers must not be restored without re-proving I2 (LA-001)
I8  WebSocket / CL+TE conflict / non-GET → Hyper fallback (not wire)
```

### CAP034_TESTS

| Test / harness | What it proves |
| --- | --- |
| `wire_conn.rs` unit tests | `encode_proxy_response_head` strips hop headers; forces close+chunked; rejects CL leak |
| `wire_proxy_response_advertises_connection_close` | Integration of encoder + streaming writer |
| `might_use_rejects_*` | Wire eligibility guards |
| `scripts/reality/cap-034-routing-host-proxy-e2e.py` | Host ranking, hostless shadowing, **LA-001** keepalive host-switch |
| `core/src/server/wire_dispatch.rs` tests | `/api/*` ∉ `epoll_keep_alive_eligible` |
| `core/tests/kd3_4_wire_proxy_test.rs` | Wire predicate smoke |

### CAP034_DOCS

| Doc | Reference |
| --- | --- |
| `docs/governance/v044-p4-proxy-path-causal-profiling.md` | ROOT_1B wire asymmetry (PROVEN) |
| `docs/governance/v044-cross-profile-performance-root-cause-review-p1-p4.md` | P4 path map |
| `scripts/reality/run-cap-034-dual-linux.sh` | Cap034 closure harness |
| `module-api/src/hop_by_hop.rs` | Hop-by-hop SoT (ADR-011 semantics) |

### CAP034_HISTORICAL_DEFECTS

| ID | Class | Resolution |
| --- | --- | --- |
| Hostless route shadowing named Host on wire | PROTOCOL_CORRECTNESS / routing | Fixed — `resolve_proxy_route` requires Host (Cap034) |
| LA-001 frozen `cluster_id` on keepalive Host switch | FAILURE_CONTAINMENT | **Closed by one-shot close** — E2E accepts `connection_closed_after_wire_admission` |
| Cap030 buffered proxy body | PERFORMANCE / streaming | Wire must stream; no CL-based full collect |
| WAF bypass on wire (WAF-LOGIC-P3-C) | SECURITY | Fixed — wire runs WAF before upstream |

### CAP034_SECURITY_DEPENDENCIES

- Hop-by-hop stripping via `module-api/src/hop_by_hop.rs::strip_hop_by_hop_headers` (shared with Hyper path)
- Wire WAF gate in `wire_dispatch.rs` before `serve_proxy_wire`
- `might_use_proxy_wire` rejects CL+TE request ambiguity and WebSocket upgrade on wire

### CAP034_PROTOCOL_DEPENDENCIES

- Cap030 streaming body contract (`hyper_forward.rs`, `wire_conn.rs`)
- Cap021 upstream connect retry on wire GET (`forward_get_for_wire`)
- Cap056 cache policy → Hyper fallback (wire must not bypass cache)

---

## 3. Response path trace (upstream head → client termination)

```text
1. read_until_headers_async          core/src/server/io.rs
2. plan_wire_decision → Proxy        core/src/kernel/wire.rs
3. resolve_proxy_route (Host rank)   core/src/server/wire_dispatch.rs
4. evaluate_wire_waf                 core/src/server/wire_dispatch.rs
5. proxy_wire::serve_proxy_wire      module-api → kernel_hooks
6. wire_conn::serve                  crates/exyonq-mod-proxy/src/wire_conn.rs
7. forward_get_for_wire              crates/exyonq-mod-proxy/src/runtime.rs
8. attempt_forward_empty_body        crates/exyonq-mod-proxy/src/hyper_forward.rs
   → strip_hop_by_hop_headers (upstream response)
9. write_proxy_response_wire
10. encode_proxy_response_head       ← WHERE_CONNECTION_CLOSE_FORCED
                                     ← WHERE_TRANSFER_ENCODING_CHUNKED_SELECTED
                                     ← WHERE_CONTENT_LENGTH_IS_REMOVED (skip in loop)
11. write_streaming_te_progressive   ← chunk encode + flush + 0\\r\\n\\r\\n
12. stream.shutdown()                ← WHERE_KEEPALIVE_ELIGIBILITY_DECIDED (terminal close)
```

**Hyper fallback path** (same P4 config if wire ineligible): `serve_connection` with
`http1.keep_alive(true)` — Hyper emits framing; upstream hop headers stripped in
`hyper_forward.rs`; **does not** use Cap034 encoder.

---

## 4. Historical reason classification

```text
CAP034_ORIGINAL_PROBLEM =
  Correct Host→cluster ranking on wire fast path without keepalive re-entry bugs (LA-001);
  avoid clients mis-reading EOF on one-shot socket; simplify downstream framing while
  Cap030 mandates streaming without upstream CL expansion.

CAP034_ORIGINAL_INVARIANT =
  ONE_ADMISSION_ONE_RANKED_PROXY_RESPONSE; downstream framing unambiguous; no silent
  hostless shadowing; no keepalive cluster freeze.

CAP034_CURRENT_INVARIANT = unchanged (see §2)

HAS_THE_ORIGINAL_CONSTRAINT_CHANGED =
  PARTIAL — Host ranking is proven closed; streaming (Cap030) remains; P4 profiling shows
  wire framing asymmetry vs rivals but does not prove Cap034 is the dominant CPU root.

CLASSIFICATION =
  PROTOCOL_CORRECTNESS (routing/Host) +
  FAILURE_CONTAINMENT (LA-001, EOF semantics) +
  SIMPLIFICATION (single downstream shape on wire) +
  NOT primarily SECURITY (though reframing is conservative)
```

---

## 5. HTTP/1.1 semantics matrix (downstream / client)

Legend: `CL` = Content-Length, `KA` = keep-alive, `CH` = chunked, `—` = N/A or not on wire path.

| # | Case | CL safe? | KA safe? | Must CH? | Must close? | Wire path today |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 200 known length | YES | YES | NO | NO | CH + close (not required) |
| 2 | 200 unknown length | NO | YES | YES | NO | CH + close |
| 3 | Streaming (SSE) | NO | YES | YES | NO | CH + close (KA possible) |
| 4 | Upstream CL pass-through | YES* | YES* | NO | NO | strips CL → CH |
| 5 | Upstream chunked | NO** | YES | YES | NO | re-chunks via Cap030 |
| 6 | Upstream close-delimited | NO | NO | YES | YES | CH + close |
| 7 | HEAD | YES (0) | YES | NO | NO | wire GET-only today |
| 8 | 1xx | — | — | — | — | not wire path |
| 9 | 204 | YES (0) | YES | NO | NO | not typical /api wire |
| 10 | 304 | YES (0) | YES | NO | NO | not wire path |
| 11 | CONNECT | — | — | — | — | Hyper only |
| 12 | Protocol upgrade | — | — | — | — | Hyper fallback |
| 13 | WebSocket | — | — | — | — | rejected on wire |
| 14 | Tunnel/raw | — | — | — | — | Hyper only |
| 15 | Upstream EOF mid-body | NO | NO | — | YES | poison connection |
| 16 | Malformed upstream | NO | NO | — | YES | poison connection |
| 17 | Retry before response committed | — | — | — | — | Cap021 upstream only |
| 18 | Retry after response head committed | NO | NO | — | YES | must not reuse |
| 19 | Body transformed by ExyonQ | NO | case-by-case | case-by-case | often YES | P4 RAW: NO transform |
| 20 | Module/WAF body filter | NO | case-by-case | — | often YES | P4 bench: WAF off |
| 21 | Compression enabled | NO | case-by-case | often CH | case-by-case | not P4 wire |
| 22 | De/recompression | NO | NO | often CH | often YES | not P4 wire |
| 23 | HTTP/1.0 client | case-by-case | implicit close | case-by-case | often YES | wire assumes HTTP/1.1 |
| 24 | HTTP/1.1 KA client | as above | as above | as above | as above | forced close |
| 25 | Upstream KA reused | — | — | — | — | Hyper client pool (upstream) |

\* Only if ExyonQ does not transform body and length is validated while streaming.  
\** Downstream may use chunked even when upstream was chunked; need not preserve upstream TE token.

**Product precedent:** static wire (`crates/exyonq-mod-static/src/wire.rs`) already emits
`Content-Length` + `Connection: keep-alive` for known bodies on eligible paths.

---

## 6. Content-Length preservation

```text
IS_UPSTREAM_CONTENT_LENGTH_TRUSTED =
  CONDITIONAL — trusted for pass-through only when body is not transformed and bytes on wire
  match declared length (Cap030 streams without pre-counting today)

UNDER_WHAT_VALIDATION =
  Stream byte count matches upstream CL OR materialized buffer with known len (Cap030 forbids
  materialize-for-CL on wire); WAF/modules/compression must be absent or accounted for

CAN_BODY_LENGTH_CHANGE_AFTER_HEADER_RECEIPT = NO on P4 wire RAW path
WHY = no WAF body filter, no compression module, no cache materialization on wire

CAN_WAF_OR_MODULES_TRANSFORM_BODY = YES in general product; NO on P4 bench wire path
CAN_PROXY_FILTERING_CHANGE_BODY = NO on P4 wire path
CAN_RETRIES_CHANGE_BODY_LENGTH = NO after downstream response started (Cap021 upstream only)

CONTENT_LENGTH_PRESERVATION_SAFE_CASES =
  - P4-class: GET pass-through, WAF off, no cache on wire, no compression, upstream 200 with
    valid single CL, body streamed without mutation, no trailers
  - HEAD with zero body
  - 204/304 with no body (if ever routed on wire)

CONTENT_LENGTH_PRESERVATION_UNSAFE_CASES =
  - Upstream chunked or CL unknown
  - SSE / event-stream (must stream CH)
  - Any module/WAF body mutation
  - Compression or cache materialization paths
  - Trailing headers / ambiguous TE
  - Error mid-body / length mismatch
```

---

## 7. Client keep-alive eligibility

```text
CLIENT_KEEPALIVE_CURRENTLY_DISABLED_BECAUSE =
  (1) EXYONQ_IMPLEMENTATION: wire_conn::serve is one-shot — no header re-read loop;
      stream.shutdown() after response (lines 565–567)
  (2) CAP034_POLICY: advertise Connection: close so KA clients do not reuse socket (lines 242–244)
  (3) EPOLL: /api/* excluded from epoll_keep_alive_eligible (wire_eligibility.rs)
  (4) LA-001: closing avoids cluster_id freeze on Host switch without full KA state machine

HTTP_PROTOCOL_REQUIRES_CLOSE = NO for known-length successful responses
EXYONQ_IMPLEMENTATION_CURRENTLY_REQUIRES_CLOSE = YES on wire path (no second request read)
CAP034_POLICY_REQUIRES_CLOSE = YES today (encoder always emits close)
```

| Response type | HTTP allows KA? | Wire today | Safe if wire evolved? |
| --- | --- | --- | --- |
| Known-length 200 | YES | close | YES with per-request Host rank + byte integrity |
| Chunked streaming | YES | close | YES with framing completion guarantees |
| Bodyless 204/304 | YES | close | YES |
| HEAD | YES | N/A (GET only) | YES if HEAD admitted |
| Error mid-body | NO | close | close (poison) |
| Upstream KA | — (upstream) | independent | independent |

---

## 8. Failure semantics (summary)

| Event | Current wire behavior | Safe KA possible? | Connection poisoned? |
| --- | --- | --- | --- |
| Upstream disconnect before headers | 502 via Hyper client | YES (no client bytes sent) | NO |
| Upstream disconnect after headers | Partial chunked body | NO | YES |
| Upstream disconnect mid-body | Truncated chunked stream | NO | YES |
| Client disconnect | Upstream may continue until error | — | client gone |
| Body shorter than CL (if CL used) | Protocol violation | NO | YES |
| Body longer than CL | Protocol violation | NO | YES |
| Chunk framing fails | Write error | NO | YES |
| Write fails after partial response | Partial response on wire | NO | YES |
| Timeout | 504 class upstream | case-by-case | if bytes sent: YES |
| Upstream retry (Cap021) | Before response committed | YES | NO |

```text
FAILURE_SEMANTICS_SUMMARY =
  Current one-shot close is fail-closed for ambiguous termination. Any evolution to KA must
  preserve: no reuse after ambiguous body end; no KA after partial response; LA-001 Host
  re-ranking on each new request in a KA loop.
```

---

## 9. Hop-by-hop header contract

```text
HOP_BY_HOP_SINGLE_SOURCE_OF_TRUTH_PRESERVED = YES
SOURCE = module-api/src/hop_by_hop.rs (strip_hop_by_hop_headers)
USED_BY = hyper_forward.rs (upstream), wire encoder (manual skip of CONN/TE/CL)
```

Cap034 wire encoder **duplicates** skip logic inline (same three header names) rather than
calling `strip_hop_by_hop_headers` on raw bytes — semantic alignment, two code paths.
Future framing policy should centralize downstream head construction.

---

## 10. Security / WAF / module contracts

```text
CL_TE_AMBIGUITY_RISK =
  LOW on current wire (downstream always CH, never CL) —
  evolution to CL requires fail-closed length validation

CONNECTION_DESYNC_RISK =
  MEDIUM if KA added without request-boundary parser parity with static wire loop —
  mitigated today by forced close

SECURITY_INVARIANTS_REQUIRED =
  - strip_hop_by_hop before downstream emit
  - no downstream CL/TE conflict
  - no KA after poisoned/incomplete body
  - WAF on wire before upstream (Cap015)
  - no request smuggling on wire (CL+TE reject on request head)
```

Relaxing Cap034 does **not** relax WAF or hop-by-hop rules. P4 bench disables WAF but product
must remain fail-closed when WAF/modules enabled.

---

## 11. Protocol normalization policy (target — review only)

```text
DOWNSTREAM_FRAMING_MUST_BE_UNAMBIGUOUS = YES
NO_CL_TE_CONFLICT = YES
NO_REUSE_AFTER_AMBIGUOUS_BODY_TERMINATION = YES

KNOWN_LENGTH_MAY_USE_CONTENT_LENGTH = TO_BE_DECIDED (YES for P4-class pass-through)
SAFE_RESPONSE_MAY_KEEPALIVE = TO_BE_DECIDED (YES with loop + LA-001 semantics)
STREAMING_MAY_USE_CHUNKED_WITH_KEEPALIVE = TO_BE_DECIDED (YES per HTTP; needs wire loop)
UPGRADE_TUNNEL_PATH = SEPARATE_CONTRACT (Hyper only — unchanged)
```

---

## 12. Expected-value analysis (bounded reasoning only)

P4P-B evidence used **only** for plausibility — not proof of Cap034 share.

| Effect of client keep-alive | Classification | Rationale |
| --- | --- | --- |
| accepts/sec reduction | MODERATE | P4 uses 100 connections with reuse; rivals still win with KA |
| TCP handshakes/sec | MODERATE | One-shot close forces new accepts per request batch |
| scheduler wakeups | UNKNOWN | 13× ctx switches vs NGINX — multi-causal; KA may help or hurt |
| allocations | SMALL | Fewer connection structs; framing unchanged |
| RPS | UNKNOWN | Not attributable to Cap034 alone |
| CPU/request | UNKNOWN | ROOT_1A/1F dominate measured gap |

| Effect of Content-Length vs chunked (1 KiB) | Classification |
| --- | --- |
| framing work | SMALL to MODERATE |
| writes/request | SMALL (chunk overhead ~hex+CRLF per frame) |
| allocations | SMALL |
| RPS / CPU | UNKNOWN |

```text
EXPECTED_RPS_GAIN_RANGE = NOT_BOUNDED
EXPECTED_CPU_GAIN_RANGE = NOT_BOUNDED
EXPECTED_CONTEXT_SWITCH_REDUCTION_RANGE = NOT_BOUNDED
EXPECTED_MEMORY_EFFECT_RANGE = SMALL (fewer short-lived conn objects) — not proven
EXPECTED_TAIL_EFFECT = UNKNOWN (P95/P99 already win vs NGINX/OLS on P4 baseline)

EXPECTED_USER_VALUE = NOT_BOUNDED
EXPECTED_MIGRATION_VALUE_EFFECT = NOT_PROVEN
```

---

## 13. Design pattern review (ADR-021 discipline — no implementation)

```text
DESIGN_PATTERN_REVIEW = COMPLETE

NO_NEW_PATTERN_ALTERNATIVE =
  Reuse static-wire patterns: keep-alive loop + CL emission + body discard between requests;
  reuse hop_by_hop SoT; introduce DownstreamFramingDecision enum (known CL | chunked stream |
  bodyless | must_close) — not a new named framework

LIKELY_RESPONSIBILITY_OWNER = exyonq-mod-proxy (wire_conn) + module-api framing contract
LIKELY_SINGLE_SOURCE_OF_TRUTH = one function: decide_downstream_framing(parts, path_class, transforms)
LIKELY_INVALID_STATES = CL+TE together; KA after poison; CL without known length
LIKELY_LIFECYCLE = per-request framing decision; per-connection KA loop optional
LIKELY_ERROR_FLOW = length mismatch → abort body + close (no KA)

ANTI_PATTERNS_REJECTED =
  - P4-only branch (OPTION_D)
  - duplicate bool flags scattered in encoder
  - benchmark-only TE/CL without product contract
```

---

## 14. Candidate contract options

### OPTION_A — KEEP_CAP034_UNCHANGED

| Dimension | Assessment |
| --- | --- |
| PROTOCOL_CORRECTNESS | PASS |
| SECURITY | PASS (conservative) |
| FAILURE_BEHAVIOR | PASS (fail-closed) |
| IMPLEMENTATION_COMPLEXITY | NONE |
| PERFORMANCE_POTENTIAL | Baseline (asymmetric vs rivals) |
| MAINTAINABILITY | PASS |
| PORTABILITY | PASS |
| RECOMMENDATION | Safe default until bounded value proven |

### OPTION_B — KNOWN_LENGTH_CONTENT_LENGTH_PLUS_SAFE_KEEPALIVE

| Dimension | Assessment |
| --- | --- |
| PROTOCOL_CORRECTNESS | PASS for P4-class only |
| SECURITY | PASS with length validation + no transforms |
| FAILURE_BEHAVIOR | Requires poison rules |
| IMPLEMENTATION_COMPLEXITY | MEDIUM (loop + byte count or trust upstream CL) |
| PERFORMANCE_POTENTIAL | UNKNOWN — needs A/B |
| MAINTAINABILITY | MEDIUM |
| PORTABILITY | PASS |
| RECOMMENDATION | Plausible **subset** of OPTION_C |

### OPTION_C — GENERAL_DOWNSTREAM_FRAMING_POLICY (preferred direction)

| Dimension | Assessment |
| --- | --- |
| PROTOCOL_CORRECTNESS | PASS if rules in §11 enforced |
| SECURITY | PASS if hop-by-hop + poison rules preserved |
| FAILURE_BEHAVIOR | IMPROVED vs blind KA |
| IMPLEMENTATION_COMPLEXITY | BOUNDED_HIGH |
| PERFORMANCE_POTENTIAL | UNKNOWN |
| MAINTAINABILITY | BEST long-term |
| PORTABILITY | PASS |
| RECOMMENDATION | **Preferred** when/if owner authorizes product WIP |

### OPTION_D — PROFILE_SPECIFIC_FAST_PATH

REJECT — benchmark-only semantics forbidden by benchmark-integrity policy.

### OPTION_E — REMOVE_CAP034

REJECT — would remove Host-ranking one-shot discipline without replacement; not evidenced.

---

## 15. Request-path cost table (P4 wire — selected stages)

| Stage | Required | CPU cost | Recoverable | Evidence |
| --- | --- | --- | --- | --- |
| accept | YES | BOUNDED | SMALL if KA | profiling |
| client header read | YES | BOUNDED | SMALL if KA | profiling |
| request parse | YES | INSUFFICIENT | — | — |
| wire-plan decision | YES | INSUFFICIENT | — | — |
| route lookup | YES | BOUNDED | — | ROOT_1E rejected |
| WAF gate | NO on P4 bench | — | — | — |
| XFF handling | YES | BOUNDED | — | profiling |
| proxy dispatch | YES | STRONGLY_SUPPORTED | NOT_BOUNDED | Hyper stack |
| cluster resolve | YES | REJECTED material | — | P4P-B |
| pool lookup | YES | INSUFFICIENT | — | — |
| upstream encode | YES | STRONGLY_SUPPORTED | NOT_BOUNDED | Hyper |
| upstream write/read | YES | STRONGLY_SUPPORTED | NOT_BOUNDED | Hyper |
| upstream parse | YES | STRONGLY_SUPPORTED | NOT_BOUNDED | Hyper |
| response-head materialize | YES | STRONGLY_SUPPORTED | BOUNDED | Cap034 encoder |
| body/frame transfer | YES | STRONGLY_SUPPORTED | BOUNDED | Cap030+CH |
| chunked framing | YES (today) | STRONGLY_SUPPORTED | BOUNDED | wire PROVEN |
| client write | YES | STRONGLY_SUPPORTED | NOT_BOUNDED | — |
| access/metrics | NO on P4 | — | — | — |
| keepalive lifecycle | YES (close) | MODERATE | BOUNDED | contract review |
| cleanup/shutdown | YES | SMALL | — | one-shot |

---

## 16. ROOT_1B refinement (from Cap034 review)

```text
ROOT_1B_CHUNKED_TE_PLUS_CONNECTION_CLOSE =
  PRODUCT_CONTRACT_ASYMMETRY + IMPLEMENTATION_SIMPLIFICATION
  NOT HTTP_MANDATE

RECOVERABLE_COST = NOT_BOUNDED without A/B
CAUSAL_CONFIDENCE_FOR_P4_GAP_SHARE = LOW (multi-root P4P-B)
```

---

## 17. Terminal

```text
V044_CAP034_PROXY_FRAMING_AND_KEEPALIVE_CONTRACT_REVIEW_STATUS = COMPLETE
CASE = C34-C

PROTOCOL_SAFE = YES (for OPTION_C with stated invariants)
SECURITY_INVARIANTS_PRESERVED = YES (under OPTION_C rules)
FAILURE_SEMANTICS_PRESERVED_OR_IMPROVED = YES (OPTION_C with poison rules)

NEXT_PRODUCT_WIP_PROPOSAL = NONE

NEXT_MEASUREMENT_WIP_PROPOSAL =
  NONE
  # Superseded by completed V044_CAP034_FRAMING_KEEPALIVE_BOUNDED_VALUE_AB (CASE AB-C34-B).
  # See docs/governance/v044-cap034-framing-keepalive-bounded-value-ab.md

GLOBAL_MIGRATION_VALUE = NOT_PROVEN
MARKET_READINESS = NOT_READY
DOCS_RECONCILED = YES
NOTION_UPDATE_REQUIRED = YES
```
