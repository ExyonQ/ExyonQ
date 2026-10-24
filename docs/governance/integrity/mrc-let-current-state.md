# MRC-LET product side-effect — current terminal state

```text
TRACK = V044_MRC_LET_PRODUCT_SIDE_EFFECT
DOCUMENT_ROLE = CURRENT_TERMINAL_STATE
INVENTORY = .exyonq-local/status/V044-MRC-LET-PRODUCT-SIDE-EFFECT-INSTANCES.jsonl
HISTORICAL_EVIDENCE = preserved under .exyonq-local/tmp/sub-let-*
FAIL_MODE = DOCUMENTATION_FOLLOWS_EVIDENCE
```

This document records **current** subroot closures and invariants. It does not
rewrite historical inventory rows or prior evidence artifacts. When a subroot
closes, append or update here; leave historical runs and superseded classifications
in their original evidence paths.

## Global side-effect success invariants (preserve)

```text
AUTHORITATIVE_SUCCESS MUST NOT PRECEDE REAL_SIDE_EFFECT_COMPLETION
```

| Domain | Invariant |
|--------|-----------|
| CHANNEL-SEND (accepted) | Redis ACK MUST occur only after real local admission (`try_send` success). Full queue → no ACK; event remains recoverable via Redis pending. |
| PROTOCOL-WRITE (accepted) | Delivery-specific access outcome (`waf_*`, `bad_request`, etc.) MUST NOT be recorded before the real write succeeds. Write Err → `response_write_failed` (or equivalent non-delivery), not delivery outcomes. |

## Current terminal bind

```text
CURRENT_TERMINAL_HEAD = cc67b2453b42f7d4b7101bb5165a4e66566a5887
CURRENT_TERMINAL_TREE = 9745c729621e2453601d26051affb21d841b75b0
SEAL_TERMINAL_HEAD = cc67b2453b42f7d4b7101bb5165a4e66566a5887
SEAL_TERMINAL_TREE = 9745c729621e2453601d26051affb21d841b75b0
B4_TERMINAL_HEAD = e0ff43b4d80450ac3b9d65632941c1c410692f25
B4_TERMINAL_TREE = 4e9663c7d5ffca14bf85f10050187290d7449bca
PRODUCT_BEHAVIOR_BIND = 41d41fbdc077d321819661f0084c9697fa86ac26
CARGO_LOCK_SHA256 = 6086f47da580fd6aa483ffb5b2c10912b2dad8eac51f25375b81228cc896bef6
CARGO_LOCK_CHANGED = NO
DEPENDENCY_BASELINE_DRIFT = NO
MRC_LET_TOTAL_INSTANCES = 245
MRC_LET_ACCOUNTED_CLOSED_INSTANCES = 245
MRC_LET_REMAINING_INSTANCES = 0
CLOSED_SUBROOTS = 28
REMAINING_SUBROOTS = 0
OPEN_PROVEN_PRODUCT_DEFECTS = 0
WORKTREE_STATUS = CLEAN (at aggregate seal close)
AGGREGATE_SEAL_STARTED = YES
AGGREGATE_SEAL = CLOSED_VERIFIED
BENCHMARK_REENTRY = YES
```

**Note:** Historical inventory rows (sealed @ `162b52c0`) retain scanner-era F
and preliminary **best-effort** labels. **Current terminal class** below
reflects rebind @ `b8ed7971`. Do not delete historical evidence artifacts.

## Observability side-effect invariant (B4 established)

```text
TELEMETRY_LOSS MUST NOT CREATE FALSE PRODUCT_SUCCESS
TELEMETRY_LOSS MUST NOT CORRUPT EXPORTER/SINK GENERATION STATE
ACCOUNTED_SINK_FAILURES → bounds::SINK_ERROR_COUNT (file/journald/syslog)
CONSOLE_FANOUT → advisory best-effort (no authoritative delivery claim)
```

Cap061 `FanoutLayer::on_event`: discarded console `write_all`/`flush` affect
only advisory console output. File writes call `note_sink_error()` on failure;
journald/syslog paths document drop accounting in `bounds.rs`. Request
admission/dispatch outcomes are independent of log line loss.

## Batch closures

### MRC-LET-B5 — CLOSED_RECONCILED_NOT_PRODUCT_DEFECT

```text
MRC_LET_B5_STATUS = CLOSED_RECONCILED_NOT_PRODUCT_DEFECT
B5_INSTANCE_COUNT = 88
B5_ACCOUNTED = 88/88
GLOBAL_INSTALL_SET = 15/15
STATE_MISC = 10/10
CLEANUP = 5/5
OTHER_MISC = 58/58
OTHER_MISC_MICROCLUSTERS = 13
PASS2_EXTRACTED_INSTANCES = 10
PROVEN_PRODUCT_DEFECTS = 0
PRODUCT_DEFECTS_FIXED = 0
UNRESOLVED = 0
PRODUCT_MUTATION = NO
B5_REBIND_HEAD = 5f8ac5e7bf53813e980f6a57de12c575c11bd1c4
B4_TERMINAL_TREE = 58dd40c4c9ac9e36c70fc1d0e9cf1a5d54a02d90
```

Two-pass heterogeneous reconciliation @ B4 authority bind. Pass 1: exact rebind +
causal triage on all 88. Pass 2: independent contract on 10 extracted outliers
(WAF OnceLock hooks, static wire fire-and-forget, Redis ACK inventory drift,
correlation getrandom, test harness `run_on`). **No proven reachable product
defect**; **no product mutation**.

Evidence: `.exyonq-local/tmp/mrc-let-b5-classifications.json`

| Subroot | Inst | Terminal disposition summary |
|---------|------|------------------------------|
| SUB-LET-GLOBAL-INSTALL-SET | 15/15 | PROVEN_RECONSTRUCTABLE_STATE (10) + PROVEN_IDEMPOTENT_DUPLICATE_SET (5) |
| SUB-LET-STATE-MISC | 10/10 | TOOLING_ONLY (1) + TEST_ONLY (1) + PROVEN_RECONSTRUCTABLE_STATE (5) + PROVEN_BEST_EFFORT (2) + PROVEN_NO_FALSE_SUCCESS (1) |
| SUB-LET-CLEANUP | 5/5 | TEST_ONLY (2) + PROVEN_SHUTDOWN_ONLY (3) |
| SUB-LET-OTHER-MISC | 58/58 | 13 micro-clusters — see below |

**Disposition totals (88):** PROVEN_RECONSTRUCTABLE_STATE 28 ·
PROVEN_INVARIANT 13 · TEST_ONLY 17 · PROVEN_SHUTDOWN_ONLY 7 ·
PROVEN_BEST_EFFORT 6 · PROVEN_IDEMPOTENT_DUPLICATE_SET 6 · TOOLING_ONLY 5 ·
FALSE_POSITIVE 2 · PROVEN_NO_FALSE_DELIVERY 2 · PROVEN_NO_FALSE_SUCCESS 1 ·
PROVEN_ADVISORY_NOTIFICATION 1

#### SUB-LET-GLOBAL-INSTALL-SET (15/15)

OnceLock / composition-root hook installs (`kernel_control_port`, `epoll_start`,
`io_uring_start`, `sync_accept_start`, `waf.rs`). **Pass 2 (WAF):** live reload
uses `ArcSwap` + staged `PENDING_BINDING`; duplicate `OnceLock::set` Err does
not leave required component uninstalled or create generation divergence.
`install_waf_runtime_binding` also writes `current_binding_slot().store(...)`.

| Disposition | Role |
|-------------|------|
| PROVEN_RECONSTRUCTABLE_STATE | First-wins platform hook registration; authoritative state via getters / ArcSwap |
| PROVEN_IDEMPOTENT_DUPLICATE_SET | WAF engine/challenge/Cap013 hook duplicate `set()` — idempotent composition root |

#### SUB-LET-STATE-MISC (10/10)

| ID | Disposition | Notes |
|----|-------------|-------|
| LET-001 | TOOLING_ONLY | `l2_coord_lab` purge — `EXYONQ_L2_COORD_LAB=1` only |
| LET-025 | PROVEN_RECONSTRUCTABLE_STATE | shutdown watch `changed().await` discard after early return |
| LET-026 | TEST_ONLY | lifecycle drain test |
| LET-027,039,040,172 | PROVEN_RECONSTRUCTABLE_STATE | init-once CAS env caches |
| LET-092,095 | PROVEN_BEST_EFFORT | Redis poison-pill XACK after decode/queue reject |
| LET-094 | PROVEN_NO_FALSE_SUCCESS | **Inventory stale @ L813** — success path now checks `xack(...).is_err()` → `note_ack_fail()` @ L822–827 |

#### SUB-LET-CLEANUP (5/5)

| ID | Disposition | Role |
|----|-------------|------|
| LET-038,186 | TEST_ONLY | test thread `join()` |
| LET-054,079,206 | PROVEN_SHUTDOWN_ONLY | worker/subscriber/pool shutdown join after abort |

#### SUB-LET-OTHER-MISC (58/58) — micro-clusters

| Cluster | Count | Disposition | Pass 2 highlights |
|---------|------:|-------------|-------------------|
| MC-OM-01 config `?` Ok discard | 1 | FALSE_POSITIVE | LET-007 `user_backend_id()?` |
| MC-OM-02 compile structural asserts | 13 | PROVEN_INVARIANT | kernel/platform contract consumers |
| MC-OM-03 integration tests | 17 | TEST_ONLY | fcgi/proxy/ws/control/wire harness |
| MC-OM-04 shutdown close/join | 4 | PROVEN_SHUTDOWN_ONLY | quiche/quinn close, listener task await |
| MC-OM-05 OnceLock hook install | 13 | PROVEN_RECONSTRUCTABLE_STATE | dispatch/static/metrics hooks |
| MC-OM-06 redis subscriber setup | 2 | PROVEN_RECONSTRUCTABLE_STATE | stream group + claim_pending |
| MC-OM-07 static wire handoff | 2 | PROVEN_NO_FALSE_DELIVERY | LET-060/182 — pool admission ≠ delivery claim |
| MC-OM-08 epoll socket best-effort | 4 | PROVEN_BEST_EFFORT | fcntl, terminal 503 send, nonblock |
| MC-OM-09 health probe drain | 1 | PROVEN_BEST_EFFORT | LET-148 body timeout drain |
| MC-OM-10 correlation getrandom | 1 | PROVEN_ADVISORY_NOTIFICATION | LET-190 — observability id only |
| MC-OM-11 header parse iterator | 1 | FALSE_POSITIVE | LET-058 `lines.next()` |
| MC-OM-12 spike harness | 4 | TOOLING_ONLY | boundary spike benches |
| MC-OM-13 rustls provider install | 1 | PROVEN_IDEMPOTENT_DUPLICATE_SET | LET-187 |

**Pass 2 contracts (material outliers — all NOT_PRODUCT_DEFECT):**

- **LET-060:** `serve_static_tcp` — blocking pool `Accepted` returns `Ok(())` =
  queued to pool, not response delivered; sync path delegates to
  `serve_wire_site_tcp` with access emission inside `wire_conn`.
- **LET-182:** `run_blocking_static_job` — errors contained in blocking thread;
  Tokio caller already completed admission contract.
- **LET-190:** getrandom failure → zero-filled correlation id; does not create
  false request success or security bypass (internal observability only).

### MRC-LET-B4 — CLOSED_RECONCILED_NOT_PRODUCT_DEFECT

```text
MRC_LET_B4_STATUS = CLOSED_RECONCILED_NOT_PRODUCT_DEFECT
B4_INSTANCE_COUNT = 8
B4_ACCOUNTED = 8/8
PROVEN_PRODUCT_DEFECTS = 0
PROVEN_BEST_EFFORT = 0
PROVEN_ADVISORY_OBSERVABILITY = 4
PROVEN_RECONSTRUCTABLE_STATE = 2
PROVEN_SHUTDOWN_ONLY = 0
PROVEN_NO_FALSE_SUCCESS = 2
TEST_ONLY = 2
FALSE_POSITIVES = 0
OTHER_RECONCILED = 0
OUTLIERS_EXTRACTED = 0
UNRESOLVED = 0
PRODUCT_MUTATION = NO
B4_TERMINAL_HEAD = 5f8ac5e7bf53813e980f6a57de12c575c11bd1c4
B4_TERMINAL_TREE = 58dd40c4c9ac9e36c70fc1d0e9cf1a5d54a02d90
PRODUCT_BEHAVIOR_BIND = 41d41fbdc077d321819661f0084c9697fa86ac26
```

Evidence: `.exyonq-local/tmp/mrc-let-b4-classifications.json`

| Subroot | Inst | Disposition |
|---------|------|-------------|
| SUB-LET-METRICS | 4/4 | TEST_ONLY (2) + PROVEN_RECONSTRUCTABLE_STATE (2) |
| SUB-LET-LOGGING | 2/2 | PROVEN_ADVISORY_OBSERVABILITY |
| SUB-LET-FLUSH | 1/1 | PROVEN_ADVISORY_OBSERVABILITY |
| SUB-LET-OBS-EXPORT-WRITE | 1/1 | PROVEN_ADVISORY_OBSERVABILITY |

#### SUB-LET-METRICS (4/4)

| ID | Expression | Role | Disposition |
|----|------------|------|-------------|
| LET-123/127 | `drain_one_fcgi_request(...).map(...)` | `#[test]` fcgi peer server | TEST_ONLY — scanner misclassified `Option::map` as metrics |
| LET-151 | `GLOBAL_METRICS.set(...)` | ProxyRuntime OnceLock install | PROVEN_RECONSTRUCTABLE_STATE — first-wins; `global_hyper_metrics()` default fallback |
| LET-167 | `install_epoll_metrics_hooks(...)` | Static kernel hook registration | PROVEN_RECONSTRUCTABLE_STATE — OnceLock first-wins @ startup |

`IS_METRIC_AUTHORITATIVE_PRODUCT_STATE = NO` for install discards; counters
themselves are advisory observability, not admission gates.

#### SUB-LET-LOGGING (2/2)

| ID | Path | Disposition |
|----|------|-------------|
| LET-191 | `install.rs` FanoutLayer text `format_fields` | PROVEN_ADVISORY_OBSERVABILITY — operational log formatting; parallel file path uses `note_sink_error` |
| LET-196 | `syslog_sink.rs` Layer `format_fields` → `emit_formatted` | PROVEN_ADVISORY_OBSERVABILITY — `emit_formatted` increments `SINK_ERROR_COUNT` on syslog I/O failure |

`IS_LOG_REQUIRED_BY_PRODUCT_CONTRACT = NO` for individual format line loss;
security/audit visibility is best-effort at emit site with error accounting on
non-console sinks per Cap061 bounds.

#### SUB-LET-FLUSH (1/1) — LET-193

Console `out.flush()` after fanout `write_all`. `FALSE_FLUSH_SUCCESS_POSSIBLE =
NO` — no API or state transition claims durable flush; `DATA_LOSS_POSSIBLE =
YES` (bounded: one log line) without affecting request path.
`PROVEN_ADVISORY_OBSERVABILITY`.

#### SUB-LET-OBS-EXPORT-WRITE (1/1) — LET-192

Console fanout `write_all(bytes)` — **not** OTLP export (OTel uses separate
batch worker per `bounds::otel_batch_bounds`). `IS_EXPORT_REQUIRED_FOR_REQUEST_SUCCESS
= NO`; `CAN_FAILURE_CREATE_FALSE_EXPORT_SUCCESS = NO`; `CAN_FAILURE_BLOCK_PRODUCT_DATA_PLANE
= NO`. `PROVEN_ADVISORY_OBSERVABILITY` + `PROVEN_NO_FALSE_SUCCESS`.

### MRC-LET-B3 — CLOSED_RECONCILED_NOT_PRODUCT_DEFECT

```text
MRC_LET_B3_STATUS = CLOSED_RECONCILED_NOT_PRODUCT_DEFECT
B3_INSTANCE_COUNT = 22
B3_ACCOUNTED = 22/22
PROVEN_PRODUCT_DEFECTS = 0
PROVEN_BEST_EFFORT = 16
PROVEN_UNUSED_OK_VALUE = 1
PROVEN_INVARIANTS = 0
PROVEN_NO_FALSE_DELIVERY_CLAIM = 2
CAUSAL_FOLDS = 0
TEST_ONLY = 5
FALSE_POSITIVES = 0
OTHER_RECONCILED = 0
OUTLIERS_EXTRACTED = 0
UNRESOLVED = 0
PRODUCT_MUTATION = NO
B3_TERMINAL_HEAD = c48863a22a733a8f610c0f6b8259f7810dfc7ca1
PRODUCT_BEHAVIOR_BIND = 41d41fbdc077d321819661f0084c9697fa86ac26
```

Rebind @ `c48863a2`: historical **best-effort** preclassification was **stale**
for six OK-discard-after-expect rows (all test/self_test paths). No reachable
false-success or panic defect proven. Socket options and epoll drain writes satisfy
the seven-condition best-effort standard.

| Subroot | Inst | Historical | Current disposition | Changed |
|---------|------|------------|---------------------|---------|
| SUB-LET-SOCKET-OPTION | 14 | BEST_EFFORT | PROVEN_BEST_EFFORT (14) | NO |
| SUB-LET-OK-DISCARD-AFTER-EXPECT | 6 | BEST_EFFORT | TEST_ONLY (5) + PROVEN_UNUSED_OK_VALUE (1) | YES |
| SUB-LET-EPOLL-DRAIN-WRITE-BEST-EFFORT | 2 | BEST_EFFORT | PROVEN_BEST_EFFORT + PROVEN_NO_FALSE_DELIVERY_CLAIM | NO |

Evidence: `.exyonq-local/tmp/mrc-let-b3-classifications.json`

#### SUB-LET-SOCKET-OPTION (14/14) — PROVEN_BEST_EFFORT

All fourteen discard `Result` from `set_*timeout`, `set_nodelay`, or
`set_reuseport`. Failure does **not** claim delivery or admission success;
worst case falls back to OS default socket behavior while the authoritative
path continues (bind with `reuseaddr`?, header read, redis cmd, fcgi checkout,
static blocking serve). Code comment @ redis `with_cmd` explicitly documents
best-effort command timeout.

| ID | Option | When | FALSE_SUCCESS |
|----|--------|------|---------------|
| LET-036/037 | nodelay + read_timeout | drain-reject / admit header read | NO |
| LET-051 | SO_REUSEPORT | listener bind (after required reuseaddr?) | NO |
| LET-082/083 | read/write timeout | redis `with_cmd` | NO |
| LET-088/089 | read/write timeout | redis invalidation loop | NO |
| LET-115–118 | read/write timeout | fcgi pool checkout/connect | NO |
| LET-133/134 | read_timeout set/restore | fcgi idle probe | NO |
| LET-181 | read_timeout | blocking static job | NO |

`REQUIRED_FOR_CORRECTNESS = NO` for all; `FAILURE_CAN_CREATE_FALSE_SUCCESS = NO`.

#### SUB-LET-OK-DISCARD-AFTER-EXPECT (6/6)

Historical best-effort label **invalid** — none are production hot-path error
swallows. Each `.expect()` audited separately.

| ID | Context | Expect target | Disposition |
|----|---------|---------------|-------------|
| LET-023 | `#[test]` wire planner | `plan_wire_decision` on synthetic head | TEST_ONLY |
| LET-184 | `#[test]` blocking pool | `client1.read` on tcp_pair | TEST_ONLY |
| LET-185 | `#[test]` keepalive timeout | `s.read` first response | TEST_ONLY |
| LET-188 | `#[test]` TLS loader | `load_certs` after empty-key Err | TEST_ONLY |
| LET-189 | `#[test]` pipeline | `transform_response().await` | TEST_ONLY |
| LET-242 | `self_test()` hook | Request builder only; `client_key` → `String` | PROVEN_UNUSED_OK_VALUE |

`CAN_EXPECT_PANIC_ON_REAL_SUPPORTED_INPUT = NO` on all product paths;
panics confined to test/self_test harness failure.

#### SUB-LET-EPOLL-DRAIN-WRITE-BEST-EFFORT (2/2) — PROVEN_BEST_EFFORT

Distinct instances from DRAIN-BOUNDARY canonical LET-057; **same causal contract**
(probe-aware drain reject, no delivery claim on write failure).

| ID | Stage | Authoritative outcome after discard |
|----|-------|-------------------------------------|
| LET-214 | `serve_epoll_fd` drain branch | `Err(ConnectionAborted, "draining")` |
| LET-215 | `sendfile_read_and_serve` drain branch | `remove_conn` → `ReadServe::Closed` |

`SUCCESS_CLAIM_AFTER_FAILURE = NO`; `CONNECTION_REUSE_POSSIBLE = NO` after branch.
Contrast: WAF reject on same file uses `if let Err` + `warn!` — drain path is
intentionally best-effort notify before mandatory close (aligned with epoll
`write_drain_boundary` family).

### MRC-LET-B2 — CLOSED_RECONCILED_NOT_PRODUCT_DEFECT

```text
MRC_LET_B2_STATUS = CLOSED_RECONCILED_NOT_PRODUCT_DEFECT
B2_INSTANCE_COUNT = 15
B2_ACCOUNTED = 15/15
WRITE_IO_MISC_INSTANCE_DELTA = 0
PROVEN_PRODUCT_DEFECTS = 0
ALREADY_FIXED_CURRENT_CONTRACT = 10
TEST_ONLY = 3
FALSE_POSITIVES = 2
UNRESOLVED = 0
PRODUCT_MUTATION = NO
TERMINAL_HEAD = c405e14727d1bf54489cf76c083d8fe5ac102dba
PRODUCT_BEHAVIOR_BIND = 41d41fbdc077d321819661f0084c9697fa86ac26
```

Rebind @ current HEAD: historical `F` / strong-defect labels were **stale** for
all 15 instances — product repairs predated B2. No new fixes required.

| Subroot | Inst | Historical | Current disposition | Changed |
|---------|------|------------|---------------------|---------|
| SUB-LET-H3-ERROR-RESPONSE-WRITE | 6 | F | ALREADY_FIXED_CURRENT_CONTRACT | YES |
| SUB-LET-WIRE-REJECT-WRITE | 2 | F | ALREADY_FIXED_CURRENT_CONTRACT | YES |
| SUB-LET-CTRL-WRITE-FALSE-OK | 1 | F | ALREADY_FIXED_CURRENT_CONTRACT | YES |
| SUB-LET-STATIC-ADMISSION-REJECT-WRITE | 1 | F | ALREADY_FIXED_CURRENT_CONTRACT | YES |
| SUB-LET-RATELIMIT-ALLOW-KEY | 3 | J/UNKNOWN | TEST_ONLY | YES |
| SUB-LET-PATH-SAFE | 2 | J/UNKNOWN | FALSE_POSITIVE | YES |

#### SUB-LET-H3-ERROR-RESPONSE-WRITE (6/6)

s2n provider (`exyonq-http3-provider-s2n` L164–212) and quinn-legacy
(`quinn_provider.rs` L114–152): error-path `write_h3_response` uses
`if let Err` + `DIAG.response_errors` / `RESPONSE_WRITE_ERRORS` + `warn!`;
**no** false delivery claim on write failure. `clean_finish` only on success path.

#### SUB-LET-WIRE-REJECT-WRITE (2/2)

`complete_wire_reject` (`wire_dispatch.rs` L111–124): write Ok →
`outcome=bad_request`; write Err → `response_write_failed` + counter. Inventory
line refs (547/747) stale — no bare discarded `write_all` on reject path.

#### SUB-LET-CTRL-WRITE-FALSE-OK (1/1)

LET-197: `write_response(...)?` at L317/L339. Test
`error_reply_write_failure_returns_err_not_ok` proves Err propagates (real
AF_UNIX socketpair, not mock).

#### SUB-LET-STATIC-ADMISSION-REJECT-WRITE (1/1)

LET-180: `shed_blocking_admission` returns `Err` on write failure; caller emits
`response_write_failed` (not `static_blocking_shed`) at `wire_dispatch.rs`
L748–757.

#### SUB-LET-RATELIMIT-ALLOW-KEY (3/3)

LET-243/244/245: all in `#[test]` — `allow_key` returns `bool`; `let _ =`
discards return after bucket **mutation** for test seeding. Not error swallowing;
`RATE_LIMIT_FAIL_OPEN_POSSIBLE = NO`; `PRODUCT_PATH_REACHABLE = NO`.

#### SUB-LET-PATH-SAFE (2/2)

LET-169/170: `let _ = relative_as_safe_path(&relative)?` — `?` propagates
`Err` before assignment; only Ok path value discarded. Confinement check runs;
`CONFINEMENT_BYPASS_POSSIBLE = NO`.

#### SUB-LET-WRITE-IO-MISC — CLOSED_BY_CAUSAL_FOLD_INTO_DRAIN_BOUNDARY

```text
SUB_LET_WRITE_IO_MISC_ORIGINAL_INSTANCES = 3
SUB_LET_WRITE_IO_MISC_ALREADY_FOLDED = 3  (LET-035, LET-052, LET-216)
SUB_LET_WRITE_IO_MISC_REMAINING = 0
SUB_LET_WRITE_IO_MISC_STATUS = CLOSED_BY_CAUSAL_FOLD_INTO_DRAIN_BOUNDARY
INSTANCE_ACCOUNTING_DELTA = 0
```

Bookkeeping seal only — instances already closed under DRAIN-BOUNDARY-WRITE.

### MRC-LET-B1 — CLOSED_RECONCILED_NOT_PRODUCT_DEFECT

```text
MRC_LET_B1_STATUS = CLOSED_RECONCILED_NOT_PRODUCT_DEFECT
B1_INSTANCE_COUNT = 71
B1_ACCOUNTED = 71/71
PROVEN_PRODUCT_DEFECTS = 0
ZERO_FAKE_VIOLATIONS = 0
OUTLIERS_EXTRACTED = 0
UNRESOLVED = 0
PRODUCT_MUTATION = NO
TERMINAL_HEAD = d1382a9ca86391f178fd5f0d0b1776e227dd0948
PRODUCT_BEHAVIOR_BIND = 41d41fbdc077d321819661f0084c9697fa86ac26
```

#### SUB-LET-SCANNER-NON-RESULT — CLOSED_RECONCILED_NOT_PRODUCT_DEFECT

```text
SUB_LET_SCANNER_NON_RESULT_STATUS = CLOSED_RECONCILED_NOT_PRODUCT_DEFECT
INSTANCE_COUNT = 62/62
PROVEN_SCANNER_NON_RESULT = 59
TOOLING_ONLY = 3
PROVEN_PRODUCT_DEFECTS = 0
```

All 62 instances have `RESULTISH = false` and `RESULT_TYPE = N/A_NON_RESULT` (56)
or `fmt::Result` from `write!` macro discards (6) — **not** I/O `Result` error
swallowing. Bindings are unused-value silencing: tuple destructure `(a, b)`,
loop variables, enum/type touch for exhaustiveness, `size_of` compile checks.

| Disposition | Count | Examples |
|-------------|-------|----------|
| PROVEN_SCANNER_NON_RESULT | 59 | `epoll_attach.rs` tuple discards; `connection_errors.rs` variant touch in `#[test]`; `wire.rs` `write!` header assembly |
| TOOLING_ONLY | 3 | `compat/nginx/src/location.rs`, `map_ir.rs`; `config/cli/src/lib.rs` (`opts.color` reserved) |

**Outliers extracted:** 0 (no fallible I/O Result found in batch).

Evidence: `.exyonq-local/tmp/mrc-let-b1-classifications.json`

#### SUB-LET-TEST-FIXTURE-WRITE — CLOSED_TEST_ONLY

```text
SUB_LET_TEST_FIXTURE_WRITE_STATUS = CLOSED_TEST_ONLY
INSTANCE_COUNT = 9/9
TEST_ONLY = 9
PROVEN_PRODUCT_DEFECTS = 0
ZERO_FAKE_VIOLATIONS = 0
```

All nine instances are inside `crates/exyonq-mod-fastcgi/src/conn_pool.rs`
`#[cfg(test)]` helpers (`respond_twice`, `stale_idle_retries_once_then_succeeds`
server thread). Role: **real local FastCGI wire peer** — accepts TCP/Unix,
reads real FastCGI records via `drain_one_fcgi_request`, responds with
`encode_record_frame` bytes. Tests exercise **real** `ConnPool::forward_once`;
the discarded writes are on the **test server thread**, not production forward path.

| ID | Role |
|----|------|
| LET-119–121 | `respond_twice` test helper writes |
| LET-124–126, 128–130 | `stale_idle_retries_once_then_succeeds` server thread writes |

**ZERO-FAKE:** NOT violated — no mock/fake product backend; real protocol framing
on real local socket. `REAL_PRODUCT_COMPONENT_REPLACED = NO`.

## Closed subroots (current)

### SUB-LET-PROTOCOL-WRITE-MISC — CLOSED_FIXED

```text
SUB_LET_PROTOCOL_WRITE_MISC_STATUS = CLOSED_FIXED
CLASSIFICATION = MIXED
INSTANCE_COUNT = 8/8
UNIQUE_ROOT_CAUSES = 4
PROVEN_PRODUCT_DEFECTS = 1 (fixed)
PROVEN_BEST_EFFORT = 4
TEST_ONLY = 3
PRODUCT_MUTATION = YES
PRODUCT_FIX_COMMIT = 57255f2026b3c4f9877db82394d1e03da828a1b8
TERMINAL_HEAD = 41d41fbdc077d321819661f0084c9697fa86ac26
TERMINAL_TREE = 56a400f9a5f50bad93e291250d66d561f800be8b
NETCUP_AMD64 = PASS_REAL_PRODUCTION
ORACLE_ARM64 = PASS_REAL_PRODUCTION
LOGIC_AUDIT = PASS
CONTRACT_ADVERSARIAL_AUDIT = PASS
OPEN_PROVEN_PRODUCT_DEFECTS = 0
```

**LET-059 (PROVEN_PRODUCT_DEFECT → FIXED):** On HTTP/1.1 wire WAF reject, a failed
`write_all` was discarded while access still recorded `waf_*`. Terminal contract:
write Ok → `waf_*`; write Err → `response_write_failed` + write-failure counter; no
false delivery claim.

**Non-defect instances (do not rewrite for scanner silence):**

| ID | Disposition |
|----|-------------|
| LET-010 | TEST_ONLY |
| LET-064 | TEST_ONLY |
| LET-065 | TEST_ONLY |
| LET-104 | PROVEN_BEST_EFFORT (H3 `recv_trailers` after body/drain — not a write) |
| LET-105 | PROVEN_BEST_EFFORT |
| LET-145 | PROVEN_BEST_EFFORT |
| LET-146 | PROVEN_BEST_EFFORT |

Evidence: `.exyonq-local/tmp/sub-let-protocol-write-misc-20260819T032226Z/`

### SUB-LET-WS-WRITE — CLOSED_TEST_ONLY

```text
SUB_LET_WS_WRITE_STATUS = CLOSED_TEST_ONLY
CLASSIFICATION = TEST_ONLY
INSTANCE_COUNT = 3/3
UNIQUE_ROOT_CAUSES = 1
PROVEN_PRODUCT_DEFECTS = 0
TEST_ONLY = 3
PRODUCT_MUTATION = NO
PRODUCT_WS_TUNNEL_DISCARDED_WRITE_DEFECT_FROM_THESE_INSTANCES = NO
TERMINAL_HEAD = 41d41fbdc077d321819661f0084c9697fa86ac26
TERMINAL_TREE = 56a400f9a5f50bad93e291250d66d561f800be8b
LOGIC_AUDIT_REQUIRED = NO (TEST_ONLY; product behavior unchanged)
CONTRACT_AUDIT_REQUIRED = NO (TEST_ONLY; product behavior unchanged)
```

All three instances are inside `crates/exyonq-mod-proxy/src/websocket.rs`
`#[cfg(test)] mod tests` → `spawn_ws_echo_upstream` (local test peer only).

| ID | Disposition | Notes |
|----|-------------|-------|
| LET-153 | TEST_ONLY | coalesced upgrade + frame write in test helper |
| LET-154 | TEST_ONLY | upgrade response write in test helper |
| LET-155 | TEST_ONLY | echo text frame write in test helper |

**Production WebSocket tunnel:** `tunnel()` uses `tokio::io::copy` with `?`
propagation. These test-helper discards are **not** production defects.

### SUB-LET-QUIC-SENDTO — CLOSED_RECONCILED_NOT_PRODUCT_DEFECT

```text
SUB_LET_QUIC_SENDTO_STATUS = CLOSED_RECONCILED_NOT_PRODUCT_DEFECT
CLASSIFICATION = MIXED_NON_DEFECT
INSTANCE_COUNT = 2/2
UNIQUE_ROOT_CAUSES = 1
PROVEN_PRODUCT_DEFECTS = 0
PROVEN_NO_FALSE_DELIVERY_CLAIM = 2
UNRESOLVED = 0
PRODUCT_MUTATION = NO
TERMINAL_HEAD = d95463352a23b35425c8e65852461f893678ad5c
TERMINAL_TREE = 2c757a47d01103a524c5f6153d86ee0b3d1ef8f8
PRODUCT_BEHAVIOR_BIND = 41d41fbdc077d321819661f0084c9697fa86ac26
LOGIC_AUDIT_REQUIRED = NO (no product behavior change)
CONTRACT_AUDIT_REQUIRED = NO (no product behavior change)
```

Both instances are pre-connection handshake UDP sends in the quiche HTTP/3
provider (`exyonq-http3-provider-quiche`). No `quiche::Connection` exists yet;
no entry is inserted into `clients`; no application/access success is recorded.
Established-connection sends (post-handshake) propagate `send_to` errors except
`WouldBlock` — see `run_blocking` send loop ~L389–406.

| ID | Disposition | Role | Notes |
|----|-------------|------|-------|
| LET-097 | PROVEN_NO_FALSE_DELIVERY_CLAIM | Version Negotiation (unsupported QUIC version) | `quiche::negotiate_version` builds VN bytes; UDP send is advisory. Send failure → client may retry Initial; no server connection state to diverge. |
| LET-098 | PROVEN_NO_FALSE_DELIVERY_CLAIM | Retry (empty address-validation token) | `quiche::retry` builds Retry bytes; token is stateless (`mint_token`). Send failure → handshake stalls; no false delivery or connection reuse. |

**Independent contract (both):** packet is not in quiche recovery state before
OS send; quiche stack does not own retransmission for these one-shot builders;
dropping `send_to` Result does not advance protocol/application state as though
delivery occurred. Contrast: post-handshake `client.conn.send()` → `send_to`
with fatal propagation.

**Provider reachability:** quiche is an optional compiled provider
(`[http3].provider = "quiche"`). Default product provider remains s2n; these
instances apply only when quiche is selected.

### SUB-LET-DRAIN-BOUNDARY-WRITE — CLOSED_RECONCILED_NOT_PRODUCT_DEFECT

```text
SUB_LET_DRAIN_BOUNDARY_WRITE_STATUS = CLOSED_RECONCILED_NOT_PRODUCT_DEFECT
CLASSIFICATION = PROVEN_NO_FALSE_DELIVERY_CLAIM
INSTANCE_COUNT = 1/1
WRITE_IO_MISC_FOLDED_COUNT = 3 (same causal contract; see below)
UNIQUE_ROOT_CAUSES = 1
PROVEN_PRODUCT_DEFECTS = 0
UNRESOLVED = 0
PRODUCT_MUTATION = NO
TERMINAL_HEAD = d1382a9ca86391f178fd5f0d0b1776e227dd0948
PRODUCT_BEHAVIOR_BIND = 41d41fbdc077d321819661f0084c9697fa86ac26
LOGIC_AUDIT_REQUIRED = NO
CONTRACT_AUDIT_REQUIRED = NO
```

**Canonical instance LET-057:** `write_drain_boundary_response` in
`core/src/server/wire_dispatch.rs` (~L193). Trigger: `try_enter()` fails during
drain **before** connection admission. Response is probe-aware via
`drain_probes::drain_boundary_response` (200 `/live`/`/health`, 503 `/ready` and
generic traffic). After write: best-effort `shutdown`, then `return` — no
routing, no access log, no delivery outcome recorded.

**Independent contract:** Pattern B — server already draining; request not
admitted; optional explanatory/probe response; write failure does not advance
delivery state; connection never enters active set; drain lifecycle does not
depend on client receiving bytes.

| Oracle field | Value |
|--------------|-------|
| FALSE_DELIVERY_SUCCESS_POSSIBLE | NO |
| FALSE_503_DELIVERY_CLAIM_POSSIBLE | NO |
| CONNECTION_REUSE_AFTER_FAILED_WRITE_POSSIBLE | NO |
| DRAIN_STATE_DIVERGENCE_POSSIBLE | NO |
| REQUEST_ADMITTED_BEFORE_REJECT | NO |
| SERVER_DRAIN_DEPENDS_ON_WRITE_SUCCESS | NO |
| PRODUCT_DEFECT_PROVEN | NO |

**WRITE-IO-MISC fold (same root cause, not separate WIP):**

| ID | Location | Fold |
|----|----------|------|
| LET-035 | `connection_executor.rs` `write_drain_boundary` | YES — `try_admit` reject → probe-aware bytes → `DrainRejected` |
| LET-052 | `server/mod.rs` `serve_hyper` admit reject | YES — same probe-aware boundary, no admission |
| LET-216 | `epoll_worker.rs` accept attach drain reject | YES — pre-admit generic 503; socket dropped, not registered |

Do **not** rewrite `let _ = write_all` for scanner silence; disposition is
contract-proven best-effort, not defect.

## Documentation reconciliation policy (from owner decision)

```text
DOCUMENTATION_RECONCILIATION = MANDATORY_PART_OF_DEFINITION_OF_DONE
```

Every material improvement must review documentation before terminal closure.
Documentation follows: real result → clean HEAD/TREE → terminal evidence → canonical
classification → documentation reconciliation (never the reverse).

## MRC-LET terminal subroot ledger (28/28)

Compact final accounting after B5. Historical scanner-era labels preserved in
inventory; **current terminal class** below.

| SUBROOT | INST | TERMINAL_STATUS | TERMINAL_DISPOSITION | PRODUCT_MUTATION | EVIDENCE_BIND |
|---------|-----:|-----------------|----------------------|--------------------|---------------|
| SUB-LET-SCANNER-NON-RESULT | 62 | CLOSED | PROVEN_SCANNER_NON_RESULT + TOOLING_ONLY | NO | B1 / d1382a9 |
| SUB-LET-TEST-FIXTURE-WRITE | 9 | CLOSED_TEST_ONLY | TEST_ONLY | NO | B1 / d1382a9 |
| SUB-LET-CHANNEL-SEND | 8 | CLOSED | CHANNEL_ACK_AFTER_ADMIT_CONTRACT | NO | pre-B5 / 662fc088 |
| SUB-LET-GENERATION-STATE | 9 | CLOSED | MIXED_NON_DEFECT | NO | pre-B5 / 662fc088 |
| SUB-LET-EPOLL-CTL | 5 | CLOSED | MIXED_NON_DEFECT | NO | pre-B5 / 662fc088 |
| SUB-LET-PUBLISH-STATE | 2 | CLOSED | MIXED_NON_DEFECT | NO | pre-B5 / 662fc088 |
| SUB-LET-DRAIN-BOUNDARY-WRITE | 1 | CLOSED | PROVEN_NO_FALSE_DELIVERY | NO | d1382a9 |
| SUB-LET-WRITE-IO-MISC | 3 | CLOSED_FOLDED | CAUSAL_FOLD → DRAIN-BOUNDARY | NO | d1382a9 |
| SUB-LET-QUIC-SENDTO | 2 | CLOSED | PROVEN_NO_FALSE_DELIVERY | NO | d9546335 |
| SUB-LET-PROTOCOL-WRITE-MISC | 8 | CLOSED_FIXED | 1 FIXED + BEST_EFFORT + TEST_ONLY | YES | 57255f20 |
| SUB-LET-WS-WRITE | 3 | CLOSED_TEST_ONLY | TEST_ONLY | NO | 41d41fbd |
| SUB-LET-H3-ERROR-RESPONSE-WRITE | 6 | CLOSED | ALREADY_FIXED_CURRENT_CONTRACT | NO | B2 / c405e147 |
| SUB-LET-WIRE-REJECT-WRITE | 2 | CLOSED | ALREADY_FIXED_CURRENT_CONTRACT | NO | B2 / c405e147 |
| SUB-LET-CTRL-WRITE-FALSE-OK | 1 | CLOSED | ALREADY_FIXED_CURRENT_CONTRACT | NO | B2 / c405e147 |
| SUB-LET-STATIC-ADMISSION-REJECT-WRITE | 1 | CLOSED | ALREADY_FIXED_CURRENT_CONTRACT | NO | B2 / c405e147 |
| SUB-LET-RATELIMIT-ALLOW-KEY | 3 | CLOSED | FALSE_POSITIVE | NO | B2 / c405e147 |
| SUB-LET-PATH-SAFE | 2 | CLOSED | FALSE_POSITIVE | NO | B2 / c405e147 |
| SUB-LET-SOCKET-OPTION | 14 | CLOSED | PROVEN_BEST_EFFORT | NO | B3 / c48863a2 |
| SUB-LET-OK-DISCARD-AFTER-EXPECT | 6 | CLOSED | TEST_ONLY + PROVEN_UNUSED_OK_VALUE | NO | B3 / c48863a2 |
| SUB-LET-EPOLL-DRAIN-WRITE-BEST-EFFORT | 2 | CLOSED | PROVEN_BEST_EFFORT | NO | B3 / c48863a2 |
| SUB-LET-METRICS | 4 | CLOSED | TEST_ONLY + PROVEN_RECONSTRUCTABLE_STATE | NO | B4 / 5f8ac5e7 |
| SUB-LET-LOGGING | 2 | CLOSED | PROVEN_ADVISORY_OBSERVABILITY | NO | B4 / 5f8ac5e7 |
| SUB-LET-FLUSH | 1 | CLOSED | PROVEN_ADVISORY_OBSERVABILITY | NO | B4 / 5f8ac5e7 |
| SUB-LET-OBS-EXPORT-WRITE | 1 | CLOSED | PROVEN_ADVISORY_OBSERVABILITY | NO | B4 / 5f8ac5e7 |
| SUB-LET-GLOBAL-INSTALL-SET | 15 | CLOSED | RECONSTRUCTABLE + IDEMPOTENT_SET | NO | B5 / 0d9ca0ef |
| SUB-LET-STATE-MISC | 10 | CLOSED | MIXED_NON_DEFECT | NO | B5 / 0d9ca0ef |
| SUB-LET-CLEANUP | 5 | CLOSED | TEST_ONLY + PROVEN_SHUTDOWN_ONLY | NO | B5 / 0d9ca0ef |
| SUB-LET-OTHER-MISC | 58 | CLOSED | 13 MICRO-CLUSTERS NON_DEFECT | NO | B5 / 0d9ca0ef |

```text
MRC_LET_TOTAL_INSTANCES = 245
MRC_LET_ACCOUNTED = 245/245
MRC_LET_REMAINING = 0
OPEN_PROVEN_PRODUCT_DEFECTS = 0
AGGREGATE_SEAL = CLOSED_VERIFIED (see V044_MRC_LET_AGGREGATE_SEAL below)
BENCHMARK_REENTRY = YES
```

## V044_MRC_LET_AGGREGATE_SEAL

```text
V044_AGGREGATE_SEAL_STATUS = CLOSED_VERIFIED
SEAL_START_HEAD = 4b3e5c1c16aebbda667b37ab7202706321a14b79
SEAL_START_TREE = 36854356f4320f43ffc340815b9f1c79b8ab6c04
DOC_BIND_STALE_IN_CURRENT_HEAD = YES (fixed 4b3e5c1c — doc showed 6dd5b6d6)
PRODUCT_BEHAVIOR_BIND_HISTORY = 41d41fbdc077d321819661f0084c9697fa86ac26
PRODUCT_MUTATION = NO
BENCHMARK_REENTRY = YES
```

### Gate results @ seal authority

| Gate | Result | Notes |
|------|--------|-------|
| Authority / clean tree | PASS | branch `release/v0.4.4-integration`; Cargo.lock SHA256 matches baseline |
| MRC-LET accounting | PASS | 245 unique IDs · 28/28 subroots · 0 duplicate · WRITE-IO-MISC fold = disposition only |
| Global clippy | PASS | `cargo clippy --workspace --all-targets -- -D warnings` exit 0 |
| ZERO-FAKE | PASS | integrity auditor `ZERO_FAKE_WORKSPACE_GATE=PASS`; project-integrity `FAKE_RESULT_GENERATION=0` |
| Integrity aggregate | RECONCILED_NO_OPEN_REAL_DEFECT | auditor exit 3 REVIEW_REQUIRED; CRITICAL=0 HIGH=0; MRC-LET 245/245 |
| Security / privacy ledger | PASS | SECURITY_BYPASS_INTEGRITY=PASS; LA-CAP054-008 enforced @ config compile |
| Dependencies | RECONCILED | lock unchanged; cargo-audit TOOL_FAIL (DB parse); deny h2 advisory pre-baseline |
| Logic consistency | PASS | global invariants preserved across B1–B5 terminal closures |
| Documentation | RECONCILED | this doc + 28-subroot ledger |
| Benchmark integrity | PASS | `BENCHMARK_INTEGRITY_GATE=PASS`; `PRODUCT_SEMANTIC_BRANCH=0` |

### ZERO-FAKE supplementary reconciliation

| Check | Raw | Reconciled |
|-------|-----|--------------|
| `exyonq-owned-mock-references-gate` | FAIL (50 hits) | Dev bench logs under `benchmarks/results-dev/` and `benchmarks/performance-contract/results/` — Docker log lines naming `mock-upstream` containers, not product mock backends |
| `zero-smoke-category-gate` | FAIL (4 paths) | `scripts/integrity/fixtures/**` — auditor regression fixtures intentionally named `*smoke*` |
| `no-smoke-as-proof-gate` | PASS | — |

**ZERO_FAKE_CURRENT_VIOLATIONS = 0** on supported product paths (integrity auditor forbidden counts all 0).

### Integrity auditor summary

```text
INTEGRITY_COMMAND = scripts/integrity/exyonq-integrity-audit.sh
INTEGRITY_EXIT = 3 (REVIEW_REQUIRED)
RAW_CRITICAL = 0
RAW_HIGH = 0
RAW_MEDIUM = 17
RAW_REVIEW = 2168
OPEN_REAL_CRITICAL_ROOT_CAUSES = 0
OPEN_REAL_HIGH_ROOT_CAUSES = 0
OPEN_SUPPORTED_PRODUCT_DEFECTS = 0
SECURITY_BYPASS_INTEGRITY = PASS
ARTIFACTS = .exyonq-local/integrity/20260819T143555Z
```

Historical REVIEW rows (OK_IGNORE propagation candidates, placeholder tokens in
tooling/scripts, BENCH_API_CACHE_PATHS documentation hits) do not override
terminal MRC-LET / B1–B5 reconciliations without new contradictory evidence.

### Open defect ledger (supported product)

```text
OPEN_SUPPORTED_SECURITY_DEFECTS = 0
OPEN_SUPPORTED_FALSE_SUCCESS_DEFECTS = 0
OPEN_SUPPORTED_PROTOCOL_DEFECTS = 0
OPEN_SUPPORTED_STATE_DIVERGENCE_DEFECTS = 0
OPEN_SUPPORTED_RESOURCE_LIFECYCLE_DEFECTS = 0
OPEN_SUPPORTED_PRIVACY_DEFECTS = 0
LA_CAP054_008_BLOCKER_STATUS = CLOSED_RECONCILED
  (config/ir fail-closed: public listen requires metrics scrape_bearer_token;
   wire path does not serve unauthenticated OpenMetrics — see LA-CAP054-008)
```

### Dependency / toolchain

```text
RUSTC_VERSION = 1.97.1
CARGO_VERSION = 1.97.1
CARGO_LOCK_SHA256 = 6086f47da580fd6aa483ffb5b2c10912b2dad8eac51f25375b81228cc896bef6
DEPENDENCY_BASELINE_DRIFT = NO
RUSTSEC_CURRENT_GRAPH_OPEN_SUPPORTED = 1
  (transitive h2 advisory via hyper stack — pre-baseline; lock unchanged)
CARGO_AUDIT = TOOL_FAIL (duplicate RUSTSEC-2026-0244 in advisory DB parser)
CARGO_DENY_ADVISORIES = FAILED (h2) / exit 0
```
