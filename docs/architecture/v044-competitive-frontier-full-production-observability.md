# Architecture — V044 Competitive Frontier full production observability

```text
WIP = V044_COMPETITIVE_FRONTIER_FULL_PRODUCTION_OBSERVABILITY
PHASE = 5
CASE = (terminal)
PRODUCT_MUTATION = YES_BOUNDED_PHASE_5_ONLY
PHASE_6_AUTHORIZED = NO
PARENT = V044_PHASE4_HYGIENE_RESIDUAL_P2_CLOSE
PARENT_CASE = P4HC-B
```

## Primary question

Can ExyonQ attach **real** Cap061/Cap054 production observability to the
competitive split-plane (CFD) **without** making observability the dataplane?

## Existing observability SSOT

| Surface | Location | Disposition |
| --- | --- | --- |
| Cap061 ADR | `docs/adr/037-observability-architecture.md` | PRESERVE |
| Cap061 leaf | `crates/exyonq-observability` | PRESERVE (control / product plane) |
| Cap054 metrics | `modules/metrics` + scrape | PRESERVE |
| Appender registry | `module-api/.../observability_runtime.rs` | PRESERVE + ownership repair |
| IR `[logging]` | `config/ir` | PRESERVE |
| CFD dataplane counters | local `Counters` only | DATAPLANE_LOCAL_PROJECTION |

```text
EXISTING_OBSERVABILITY_SSOT = Cap061 + Cap054 + IR [logging]
PRESERVE = Cap061 semantics, field contracts, redaction, rotation policy, Cap054 names
PRESERVE_WITH_ADAPTER = CFD → Cap061-compatible JSON / OpenMetrics projection
MOVE_TO_CONTROL_PLANE = OTLP HTTP protobuf exporters (Cap061 otel feature pulls Tokio)
DATAPLANE_LOCAL_PROJECTION = shard-local counters + bounded event ring + export thread
REPLACE_EXECUTION_ONLY = Prometheus appender ownership (test-clear must not wipe production)
RETIRE = none
```

### Why Cap061 crate is not linked into `exyonq-dataplane`

`exyonq-observability` → `exyonq-config-ir` → Hyper/Tokio even with `otel` disabled.
Linking Cap061 into the competitive dataplane would violate:

```text
TOKIO_IN_DATAPLANE = 0
HYPER_IN_DATAPLANE = 0
```

Therefore Cap061 **execution** of OTLP stays on the product/control plane.
CFD speaks Cap061 **semantics** via a tokio-free local projection + export thread.

## ADR-021 — architecture selection (Phase 5 observability)

Compared before mutation:

| Option | Summary | Verdict |
| --- | --- | --- |
| **A** | Per-request cross-process event messages | **REJECT** — reintroduces control RPC / hot-path coupling |
| **B** | Shard-local counters + bounded event ring + periodic publication | **SELECT** (core) |
| **C** | Shared-memory counters / snapshots | DEFER — portability + lifetime cost not justified for Phase 5 |
| **D** | Per-shard lock-free / single-owner aggregation | **SELECT** (with B) — shard thread owns mutable counters |
| **E** | Control-plane scrape/poll of immutable snapshot | **SELECT** (with B) — `/metrics` + optional unix event drain |
| **F** | Other | N/A |

```text
SELECTED = B + D + E
NO_NEW_PATTERN_ALTERNATIVE = Cap061/Cap054 remain SSOT; CFD projects into them
REJECTED_A_REASON = PER_REQUEST_CONTROL_PLANE_RPC must stay 0
```

### Hot-path contract

```text
PER_REQUEST_CONTROL_PLANE_RPC = 0
PER_REQUEST_CROSS_PROCESS_RPC = 0
GLOBAL_MUTEX_ON_REQUEST_PATH = 0
BLOCKING_FILE_IO_ON_REQUEST_PATH = 0
BLOCKING_SYSLOG_IO_ON_REQUEST_PATH = 0
BLOCKING_JOURNALD_IO_ON_REQUEST_PATH = 0
BLOCKING_OTLP_HTTP_ON_REQUEST_PATH = 0
NETWORK_EXPORT_ON_REQUEST_PATH = 0
```

Request path: Relaxed atomics / shard-local `u64` + `try_push` to a **bounded** ring.
Export thread / scrape socket: all sink I/O.

### Saturation

```text
OBS_EVENT_QUEUE_BOUND = 4096 events (process-wide)
OBS_SATURATION_POLICY = DROP_NEWEST_WITH_COUNTER
OBS_DROPPED_EVENT_COUNTER = exyonq_cfd_obs_events_dropped_total
```

### Metric cardinality

No raw path / Host / client IP / UA / request id labels on CFD scrape output.
Status class is bounded (`2xx|3xx|4xx|5xx|other`). WAF decision is `allow|deny|error`.

## Modes (production, not bench-secret)

| Mode | Meaning |
| --- | --- |
| O0 | Metrics scrape only (default if no sink env set) |
| O1 | O0 + console JSON + structured access/error events |
| O2 | O1 + file(+rotation) and/or syslog and/or journald as configured |

OTLP remains Cap061 product-plane export (not a dataplane dependency).

## Production OTLP data path (Phase-5 reality close answer)

```text
CFD_OBS → Cap061 OTLP HTTP protobuf exporter = NONE (by design)
```

Exact product-plane path (independent of CFD):

1. `cli/exyonq` → `exyonq_observability::install_from_config`
2. `crates/exyonq-observability` otel layer → OTLP HTTP protobuf `SpanExporter`
3. Real listener proof: `tests/otlp_http_real_listener.rs` (`--features otel`) → PASS

There is **no** CFD → Cap061 OTLP bridge, no per-request cross-process RPC, and no
Tokio/Hyper in the dataplane binary for observability.

## Phase-5 reality close status

```text
FOLLOW_ON_WIP = V044_PHASE5_OBSERVABILITY_REALITY_AND_FRONTIER_CLOSE
CASE = P5RC-C
PRODUCT_REPAIR_REQUIRED = YES
  P0: proxy/error paths under-instrumented (obs_note not on finish_response)
  P1: events_exported vs sink success; silent export-thread spawn failure
SYSLOG/JOURNALD/ROTATION Netcup E2E = PASS (instrumented event classes)
PHASE_6_AUTHORIZED = NO
```

## Integrity

```text
ZERO_FAKE = required
NO_SMOKE_AS_TERMINAL = required
ANTI_KOBAYASHI = no hidden OBS-off bench path
```

## Phase-5 accounting instrumentation repair

See `docs/governance/v044-phase5-observability-accounting-instrumentation-repair.md` (P5OR-B). Four MUST_FIX defects closed; frontier/Phase-6 not resumed.
