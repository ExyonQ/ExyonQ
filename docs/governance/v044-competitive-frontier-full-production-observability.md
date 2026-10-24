# Governance — V044 Competitive Frontier full production observability

```text
WIP = V044_COMPETITIVE_FRONTIER_FULL_PRODUCTION_OBSERVABILITY
PHASE = 5
PARENT = V044_PHASE4_HYGIENE_RESIDUAL_P2_CLOSE
PARENT_CASE = P4HC-B
PRODUCT_MUTATION = YES_BOUNDED_PHASE_5_ONLY
PHASE_6_AUTHORIZED = NO
CANONICAL_MERGE = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION = NO
```

## Architecture selection (ADR-021 / Phase 5)

```text
SELECTED = B + D + E
  shard-local counters + bounded event ring + periodic/async export
  + single-owner shard aggregation
  + scrape/poll of OpenMetrics snapshot
REJECTED_A = per-request cross-process event RPC
CAP061_SSOT = PRESERVE (semantics)
CAP061_OTLP_EXECUTION = product/control plane (Tokio via config-ir)
CFD_DATAPLANE = Cap061-semantic local projection (no Cap061 crate link)
```

Rationale: linking `exyonq-observability` into the competitive dataplane pulls
Tokio/Hyper through `exyonq-config-ir`, violating `TOKIO_IN_DATAPLANE=0`.

## Hot-path contract

```text
PER_REQUEST_CONTROL_PLANE_RPC = 0
PER_REQUEST_CROSS_PROCESS_RPC = 0
GLOBAL_MUTEX_ON_REQUEST_PATH = 0   # event ring Mutex only on try_push (bounded, non-blocking drop)
BLOCKING_SINK_IO_ON_REQUEST_PATH = 0
```

## PROMETHEUS_APPENDERS race

```text
PROMETHEUS_APPENDERS_RACE_REPRODUCED = YES (Phase-4 ambient; multi-crate parallel clear)
ROOT_CAUSE = clear_for_tests wiped production Vec mid-assert under parallel crates
FIX = production Vec never cleared; test overlay + RAII gate
PROMETHEUS_APPENDERS_RACE_FINAL = FIXED_AND_VERIFIED (pending workspace gate)
```

## Cap061 reconciliation

```text
CAP061_SEMANTICS_PRESERVED = YES (JSON fields, rotation, syslog, journald, metrics names)
CAP061_EXECUTION_ADAPTED_TO_SPLIT_PLANE = YES (CFD projection + product OTLP)
CAP061_REGRESSIONS = none intended
```

## Modes

| Mode | Sinks |
|------|-------|
| O0 | OpenMetrics scrape (if listen env set) / counters only |
| O1 | + console JSON events |
| O2 | + file(+size rotate) and/or syslog and/or journald |

Env knobs: `EXYONQ_CFD_OBS_*` (see operations doc).

## Integrity

```text
ZERO_FAKE = required
NO_SMOKE_AS_TERMINAL = required
ANTI_KOBAYASHI = no hidden OBS-off bench path
PHASE_6_AUTHORIZED = NO
```

## Phase-5 reality + frontier close (follow-on WIP)

```text
WIP = V044_PHASE5_OBSERVABILITY_REALITY_AND_FRONTIER_CLOSE
PARENT_CASE = CFO5-B
CASE = P5RC-C
CASE_MEANING = PRODUCT_REPAIR_REQUIRED
PRODUCT_MUTATION_IN_CLOSE = NO
PHASE_6_AUTHORIZED = NO
```

### Real sink evidence (Netcup amd64, run `p5rc-20260829T110530Z`)

```text
SYSLOG_REAL_E2E = PASS
  independent journalctl -t exyonq-cfd -o verbose shows
  _TRANSPORT=syslog + SYSLOG_IDENTIFIER=exyonq-cfd + JSON MESSAGE
JOURNALD_REAL_E2E = PASS
  same identifier with _TRANSPORT=journal
FILE_ROTATION_REAL = PASS (ROTATED_ARTIFACT_COUNT=4 + active jsonl)
OPENMETRICS_PARSE = PASS (instrumented paths)
OTLP_FROM_CFD = NONE_BY_DESIGN (Cap061 product-plane only)
OTLP_PRODUCT_PLANE_LISTENER = PASS (otlp_http_posts_to_real_listener + wire decode)
PROMETHEUS_APPENDERS_RACE_RECHECK = PASS (--test-threads=32)
```

### Blocking defects (independent logic audit — no silent repair)

```text
P0 = proxy success / many error paths never call obs_note (shard.rs)
     → OpenMetrics request totals undercount real proxy traffic
P1 = events_exported increments without per-sink success (export.rs)
P1 = spawn_export_thread failure silently ignored (soft OBS-off)
P1 = several OpenMetrics gauges never mutated
PRODUCT_REPAIR_REQUIRED = YES
O0_O1_O2_PERF = NOT_RUN (stopped before campaign)
WORKSPACE_FMT_CHECK = FAIL (formatting drift)
WORKSPACE_CHECK/CLIPPY/TEST = NOT_COMPLETED_DUE_TO_STOP
```

Evidence roots:

- `.exyonq-local/evidence/phase5-observability-reality-frontier-close/p5rc-20260829T105602Z/`
- `.exyonq-local/evidence/phase5-observability-reality-frontier-close/p5rc-20260829T110530Z/`

Owner must authorize a **bounded product-repair WIP** before P5RC-A/B can be claimed.
Phase 6 remains unauthorized.

## Phase-5 accounting instrumentation repair

See `docs/governance/v044-phase5-observability-accounting-instrumentation-repair.md` (P5OR-B). Four MUST_FIX defects closed; frontier/Phase-6 not resumed.

## Phase-5 resume close (P5RFR) — 2026-08-29T17:09Z

- WIP: `V044_PHASE5_OBSERVABILITY_REALITY_AND_FRONTIER_CLOSE_RESUME`
- Case: **P5RFR-B** (correctness/operability PASS; frontier YELLOW — rival CPU us/req not measured)
- Parent repair: P5OR-B; OPEN_P0=0 OPEN_P1=0
- Cap061 fresh (binary after rsync rebuild): PASS (syslog/journald/rotation/openmetrics/gauges/privacy/FD)
- CFD→OTLP hot path: NO; Cap061 OTLP product-plane listener test: PASS
- O0/O1/O2: measured; PROFILE_A O1/O2 RPS 'gains' labeled noise; PROFILE_B shows real O1 RPS/CPU cost
- Frontier vs OLS PROFILE_A: RPS +31.6% (target ≥9.3% PASS); CPU gate NOT_MEASURED → YELLOW
- Phase 6 / P1–P14: NOT AUTHORIZED
- Evidence: `.exyonq-local/evidence/phase5-observability-reality-frontier-resume/p5rfr-20260829T121934Z/`

## Phase-5 CPU frontier accounting close (supplemental) — 2026-08-29T20:17Z

```text
WIP = V044_PHASE5_CPU_FRONTIER_ACCOUNTING_CLOSE
PARENT_CASE = P5RFR-B
CASE = P5CPU-A
PRODUCT_MUTATION = NO
PHASE_6_AUTHORIZED = NO
```

- Fresh symmetric Profile-A O0 CPU µs/req on Netcup amd64 (cpuset 0-7, 7 interleaved reps).
- Primary: `/proc` utime+stime sum over complete serving PID family / successful requests.
- Secondary: host cgroup v2 `usage_usec` for containers (recon ≤0.2% medians); ExyonQ host-PID secondary NA.
- Medians: ExyonQ 13.130 · OLS 23.791 · NGINX 28.387 · HAProxy 62.496 µs/req.
- Advantage vs best rival (OLS): **+44.81%** (owner target ≥22.9% → PASS).
- Independent audit PASS; ZERO_FAKE / NO_SMOKE / ANTI_KOBAYASHI PASS.
- `PHASE5_FIVE_AXIS_FRONTIER = GREEN_CANDIDATE_FOR_OWNER_ADMISSION` (does not rewrite P5RFR-B).
- Evidence: `.exyonq-local/evidence/phase5-cpu-frontier-accounting-close/p5cpu-20260829T194956Z/`
- Performance narrative: `docs/performance/v044-competitive-frontier-full-production-observability-budget.md`

## Pingora 0.8.1 technical comparator (addendum) — 2026-08-29T20:54Z

```text
PINGORA_COMPARATOR = AUTHORIZED_SUPPLEMENTAL
PINGORA_OWNER_GATE_MEMBER = NO
PINGORA_VERSION = 0.8.1
PRODUCT_MUTATION = NO
PHASE_6_AUTHORIZED = NO
```

- Same Netcup host/cpuset/Profile-A/loadgen/CPU method as P5CPU-A; 7×30s after equivalence + upstream causal proofs.
- Pingora median: **48.993 µs/req** · RPS 32916 · CV 0.024 (7/7 valid).
- ExyonQ CPU advantage vs Pingora: **+73.20%** → `PINGORA-XA` (clearly more CPU efficient).
- Expanded best CPU rival remains **OLS** (Pingora does **not** become expanded frontier).
- Original owner gate **unchanged**: PASS vs OLS +44.81%.
- Combined terminal: `.exyonq-local/evidence/phase5-cpu-frontier-accounting-close/p5cpu-20260829T194956Z/COMBINED_TERMINAL.json`

## Upstream pool reality reconciliation — 2026-08-29T22:30Z

```text
WIP = V044_UPSTREAM_POOL_REALITY_RECONCILIATION
CASE = UPRR-C
PRODUCT_MUTATION = NO
PHASE_6_AUTHORIZED = NO
```

- Verified: Profile-A / CFD path has **client keepalive YES**, **upstream reuse NO**.
- `UpstreamPool::take` reachable; `UpstreamPool::put` dead (Phase-4 fail-closed trailing-byte race).
- Phase-2 pool `put` historically exercised (CFD-L06); hit-rate / connect≪requests **not** proven.
- Benchmark frontiers **UNCHANGED** (asymmetry is anti-ExyonQ).
- Record: `docs/governance/v044-upstream-pool-reality-reconciliation.md`
- Evidence: `.exyonq-local/evidence/upstream-pool-reality-reconciliation/uprr-20260829T223058Z/`
