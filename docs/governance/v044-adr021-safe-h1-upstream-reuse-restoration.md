# ADR-021 — Safe H1 upstream reuse restoration (binding)

```text
WIP = V044_SAFE_H1_UPSTREAM_REUSE_RESTORATION
ADR = ADR-021
STATUS = DESIGN_BINDING_IMPLEMENTED
PRODUCT_MUTATION = YES_AFTER_ACCEPT
SUPERSEDES = docs/governance/v044-adr021-upstream-pool-pre-repair-analysis.md (analysis-only; IMPLEMENT=NO)
PRESERVES = UPRR-C / UPRR-R6 (historical discard-only truth until this WIP proves otherwise)
```

## 1. Problem

Competitive CFD Profile-A keeps **client** keepalive but always **DISCARD**s upstream sockets after a successful response (`finish_response` ignores `reuse_ok`). Phase-2 historically had reachable `put`; Phase-4 fail-closed removed success-path checkin because a non-blocking trailing-byte probe can race peer writes. Benchmarks remain valid under anti-ExyonQ asymmetry (EXT-A / UPRR).

Core problem is not “call `put`”; it is proving a completed upstream connection is **clean, complete, and safe** to idle-pool.

## 2. Binding current truth (pre-mutation)

```text
CLIENT_KEEPALIVE = YES
UPSTREAM_KEEPALIVE = NO
POOL_TAKE_REACHABLE = YES
POOL_PUT_REACHABLE = NO
SUCCESS_DISPOSITION = DISCARD
CHUNKED_UPSTREAM = FAIL_CLOSED_UNSUPPORTED
EOF_FRAMED_RESPONSE = NEVER_REUSE
```

Evidence grades (research): CURRENT_DISCARD_ONLY = SOURCE_PROOF; Phase-4 motive = DOCUMENT/INFERENCE; trailing fixture = discard-isolation under discard posture, **not** put-path safety.

## 3. Decision

```text
CHOSEN_DESIGN = F_HYBRID
  = disposition-at-complete
  + dual-boundary probes (checkin + checkout)
  + idle READABLE watch (any readiness ⇒ DISCARD)
  + generation tag / flush
  + hold READABLE from take through first request-write byte
NO_NEW_PATTERN = YES
  (extend UpstreamPool + finish_response + shard poll dispatch; no Tokio/Hyper; no global pool)
ALTERNATIVES_REJECTED =
  A alone (checkin probe only) — residual idle/trailing race
  D (permanent discard) — valid fallback if ACCEPT fails; not preferred if F can be proven
  E idle-probe-only without disposition — incomplete
  C full typestate framework — unnecessary if disposition enum + RAII-ish drop-to-discard suffices
```

## 4. Lifecycle (normative)

```text
NEW → CONNECTED → IN_USE → RESPONSE_HEADERS → RESPONSE_BODY
  → RESPONSE_COMPLETE → REUSE_VALIDATION → REUSABLE | POISONED
REUSABLE → IDLE_POOLED (only after validation)
IDLE_POOLED → (idle READABLE|HUP|ERROR) → DISCARDING → CLOSED
IDLE_POOLED → CHECKOUT_VALIDATION → IN_USE | DISCARDING
POISONED → DISCARDING → CLOSED

FORBIDDEN: IN_USE → IDLE_POOLED without RESPONSE_COMPLETE + REUSE_VALIDATION
```

## 5. Reuse eligibility (all required)

```text
RESPONSE_HEADER_COMPLETE = YES
RESPONSE_FRAMING_VALID = YES          # known Content-Length or bodyless status/method
EXPECTED_BODY_COMPLETE = YES
NO_UNREAD_APPLICATION_BYTES = PROVEN  # dual probe + idle watch
NO_TRAILING_BYTES = PROVEN_TO_STANDARD
CONNECTION_NOT_POISONED = YES
UPSTREAM_NOT_EOF_AS_FRAMING = YES     # EOF-as-body-end ⇒ never reuse
NO_TIMEOUT = YES
NO_PROTOCOL_ERROR = YES
GENERATION_COMPATIBLE = YES
POOL_KEY_COMPATIBLE = YES             # SocketAddr (+ future TLS identity)
REUSE_POLICY_ALLOWS = YES             # !Connection: close
```

Bodyless (HEAD / 204 / 304): reusable when `!Connection: close` and framing complete (fix Phase-4 over-conservative `!bodyless` gate).

Chunked: remains **UNSUPPORTED / FAIL_CLOSED** — do not expand scope for reuse.

## 6. Dual-boundary + residual TOCTOU (normative)

### Checkin

1. Deregister active upstream from client map.
2. Non-blocking `read(1)`: byte/EOF/err ⇒ DISCARD; `WouldBlock` ⇒ continue.
3. Register idle token **READABLE** on shard `Poll` **before** entry is take-eligible.
4. Second non-blocking `read(1)` (closes edge-triggered probe→register gap): any byte/EOF/err ⇒ deregister + DISCARD.
5. Tag idle entry with `generation_id` + `idle_since`.

### Idle

- Explicit poll dispatch for idle-pool tokens (not silent Keep).
- READABLE | ERROR | HUP ⇒ remove + DISCARD.
- TTL / max-per-target eviction unchanged in spirit (60s / 32).

### Checkout

1. Remove from idle map; deregister idle watch.
2. Reject generation mismatch; flush entire shard idle map on generation bump.
3. Mandatory non-blocking probe: dirty ⇒ DISCARD + fresh connect.
4. **Hold READABLE** on the live upstream token from take through **first request-write byte**; readable-before-write ⇒ DISCARD (no pushback of consumed bytes).
5. Do not hand out a conn with a pending readable readiness.

### Absolute security rule

```text
UNKNOWN | DIRTY | POISONED | TRAILING | INCOMPLETE | GEN_MISMATCH | PROTOCOL_AMBIGUITY = DISCARD
```

No performance goal may weaken this.

## 7. Poison model

Poison (never put): partial write; timeout; partial/over/under body vs CL; parse failure; unexpected EOF; trailing/over-read; client abort while upstream incomplete/uncertain; generation incompatibility; probe ambiguity; idle readability; checkout-before-write readability.

## 8. Concurrency / dataplane invariants

```text
SHARD_LOCAL_POOL = YES
TOKIO_THREADS_IN_DATAPLANE = 0
HYPER_ON_COMPETITIVE_H1_PATH = 0
CONTROL_PLANE_RPC_PER_REQUEST = 0
CROSS_PROCESS_RPC_PER_REQUEST = 0
NEW_GLOBAL_HOT_LOCK = 0
TcpStream ownership = move-only (no Arc<Mutex<TcpStream>>)
```

## 9. Kill switch / rollback

```text
KILL_SWITCH = finish_response always discard (ignore reuse_ok); stop idle register
ROLLBACK = revert put call sites; restore Phase-4 discard-only comments
```

## 10. Evidence contract (post-mutation, before SAFE claims)

```text
DIRTY_CONNECTION_REUSE_COUNT = 0          # instrumented under adversarial sockets
TRAILING_BYTE_ADVERSARIAL = PASS          # delayed / checkout→write inject / HTTP-looking prefix
REAL_TCP_REUSE = PASS                     # nonce distinct; connects << requests when clean
PHASE4_TRAILING_FIXTURE = KEEP_AND_EXTEND
ZERO_FAKE = ABSOLUTE
NO_SMOKE = ABSOLUTE
NETCUP_AMD64 = REQUIRED for runtime reuse claims
DARWIN = LOCAL_ITERATION_ONLY
ORACLE_ARM64_REQUIRED = NO                # unless socket semantics diverge (revisit if proven)
```

Historical P5CPU-A / EXT-A remain valid **pre-repair** evidence; post-repair numbers are a new product state (do not overwrite).

## 11. Acceptance gates before product mutation

```text
VERTEX_ARCHITECT_VERDICT_REQUIRED = ARCHITECTURE_ACCEPT
SECURITY_ENGINEER_VERDICT_REQUIRED = SECURITY_ACCEPT
GRC = CONDITIONAL_PASS (ADR write OK; mutation only after re-ACCEPT)
CONDITIONAL_ACCEPT ≠ MUTATION_AUTHORIZED
```

## 12. Allocation budget (target)

```text
NEW_PER_REQUEST_ALLOCATIONS = 0_preferred (idle token map entry only on put)
NEW_GLOBAL_HOT_LOCK = 0
NEW_ARC_ON_STREAM = 0
```

## 13. Migration value (prospective)

Safe upstream reuse increases migration value for remote upstreams, ephemeral ports, FD churn, and future upstream TLS — **PARTIAL→YES** only after REAL_TCP_REUSE proof; not claimed pre-mutation.
