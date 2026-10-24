# Governance — Phase 4 hygiene + residual P2 close

```text
WIP = V044_PHASE4_HYGIENE_RESIDUAL_P2_CLOSE
PARENT = V044_PHASE4_BUILD_REALITY_INTEGRITY_CLOSE / P4RI-B
PRODUCT_MUTATION = YES (fmt + L4-P2-001 absolute header deadline + DriveCtx clippy)
ARCHITECTURE_MUTATION = NO
PHASE_5_AUTHORIZED = NO
PUSH = NO
TAG = NO
RELEASE = NO
NOTION_RECONCILIATION_REQUIRED = YES
```

## Case

```text
CASE = P4HC-B
PHASE4_TERMINAL_TRUST = HIGH
```

Hygiene gates closed. L4-P2-001 repaired with real-TCP slowloris evidence.
Three P3 findings remain as **documented non-blocking** (not MUST_FIX).

## Build hygiene

| Gate | Result |
|------|--------|
| `cargo fmt --all -- --check` | PASS |
| `cargo check --workspace --all-targets` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| `cargo clippy -p exyonq-cfd-dataplane -p exyonq-cfd-control -p exyonq-cfd-gen --all-targets -- -D warnings` | PASS |
| CFD Phase4 / smuggling / slowloris + SDP/control/gen tests | PASS |
| `cargo test --workspace --all-targets` (default parallel) | FAIL — ambient only |

### Workspace test ambient (proven irrelevant to this WIP)

```text
FAILING_TEST = exyonq-metrics::tests::runtime_append_includes_custom_counter_when_registered
ROOT_CAUSE = process-global PROMETHEUS_APPENDERS Mutex race under parallel multi-crate test
TOUCHED_BY_THIS_WIP = NO
ALONE_WITH_TEST_THREADS_1 = PASS
AFFECTED_CRATES_MATRIX = PASS (cfd-dataplane/control/gen + proxy-sdp)
```

Do not treat the metrics appender flake as a Phase-4 product defect.
## ADR-021 — `drive_session` too_many_arguments

```text
ADR021_CLASS = B_MISSING_CONTEXT_STATE_OBJECT
CLIPPY_DISPOSITION = REAL_ARCHITECTURAL_CLEANUP
```

`DriveCtx` bundles shard-local mutable handles. `next_token` remains a separate
`&mut Token` to avoid `&mut &mut Token` when destructuring. No `#[allow]`, no
crate exclusion, no lint policy change.

## L4-P2-001 — absolute header deadline

```text
L4_P2_001_REPRODUCED = YES (pre-fix: since reset on every readable drip)
L4_P2_001_FINAL_CLASSIFICATION = REAL_PRODUCT_RESOURCE_EXHAUSTION_DEFECT
L4_P2_001_MUST_FIX = YES → FIXED
ABSOLUTE_HEADER_DEADLINE = YES (monotonic Instant since; CLIENT_HEADER_TIMEOUT = 5s)
HEADER_IDLE_TIMEOUT = N/A for header acquisition (absolute budget only)
```

Contract: once header acquisition starts (`ReadClientHeaders.since`), the deadline
does **not** refresh on drip bytes. Transition from `Idle` starts a fresh `since`.

Evidence: `crates/exyonq-cfd-dataplane/tests/slowloris_header_deadline.rs`
(real TCP drip; concurrent healthy GETs).

## P3 ledger (non-blocking)

| ID | Classification | MUST_FIX_NOW | Notes |
|----|----------------|--------------|-------|
| L4-P3-001 | DOCUMENTED_NONBLOCKING | NO | `bytes_out` undercount after `RelayBuf` clear — metrics only |
| L4-P3-002 | DOCUMENTED_NONBLOCKING | NO | `force_client_close` always false; upstream `Connection: close` correctly poisons **upstream reuse** via hop-by-hop filter; client KA is independent (intentional reverse-proxy semantics). No request mis-association on supported CL path. |
| L4-P3-003 | DOCUMENTED_NONBLOCKING | NO | `MAX_CHUNK_*` unused because chunked TE is fail-closed by design |

## Integrity

```text
ZERO_FAKE = PASS
NO_SMOKE = PASS
ANTI_KOBAYASHI = PASS
PHASE_5_FEASIBLE = YES
PHASE_5_AUTHORIZED = NO
```
