# V044 Phase-5 — Observability accounting / instrumentation repair

```text
WIP = V044_PHASE5_OBSERVABILITY_ACCOUNTING_INSTRUMENTATION_REPAIR
PARENT = V044_PHASE5_OBSERVABILITY_REALITY_AND_FRONTIER_CLOSE / P5RC-C
CASE = P5OR-B
PRODUCT_MUTATION = YES_BOUNDED_TO_4_MUST_FIX
O0_O1_O2 = NO
PHASE_6 = NO
```

## Binding defects (closed)

| ID | Severity | Repair |
|----|----------|--------|
| OBS-P5-P0-001 | P0 | `obs_finalize` once-per-request on `Conn` for all terminal outcomes |
| OBS-P5-P1-001 | P1 | `events_exported` only when every *configured* event sink accepts (open-fail blocks) |
| OBS-P5-P1-002 | P1 | `export_spawn_outcome`: OBS env configured ⇒ spawn failure is startup-fatal |
| OBS-P5-P1-003 | P1 | Wire `active_connections`, `generation_current`, `queue_high_water` |

## Cap061 semantics preserved

```text
CAP061_SEMANTICS_PRESERVED = YES
OBS_EVENT_QUEUE_BOUND = 4096
OBS_SATURATION = DROP_WITH_COUNTER
PER_REQUEST_BLOCKING_EXPORT = 0
OTLP_FROM_CFD = NONE_BY_DESIGN
```

## Rejected alternatives

- Scatter-only `obs_note` at every return (drift / miss risk)
- Conn-lifetime RAII (wrong unit under keepalive)
- Counting export success on any live sink when another configured sink failed open
- Silent spawn ignore when OBS env is set

## Residual (nonblocking)

- Metrics scrape *sub-thread* spawn/bind failure remains non-fatal (P2). Main export thread start is fail-closed when OBS is configured.

## Evidence

`.exyonq-local/evidence/phase5-observability-accounting-instrumentation-repair/p5or-20260829T115017Z/`

## Stop

Do **not** auto-resume Phase-5 frontier (O0/O1/O2) or Phase 6. Owner authorization required.
