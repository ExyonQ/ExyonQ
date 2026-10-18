# V044 SDP-P1 — GET known-length product slice

```text
WIP = V044_SPECIALIZED_PROXY_DATAPLANE_SDP_P1_GET_KNOWN_LENGTH
CASE = SP1-C
STATUS = COMPLETE
INTEGRATION_VALUE_GATE = FAIL (+2.94% RPS < 15%)
STOP_ARCHITECTURE_INTEGRATION = YES
FEATURE_GATE = EXYONQ_SDP_P1=1 (default OFF)
SHARDS = EXYONQ_SDP_SHARDS (else cpus/2 when cpus≥4)
STACK = ADR-021 C + SD-B B (mio + httparse + socket2)
PROTOTYPE_COPY = FORBIDDEN
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
```

## Product path

When `EXYONQ_SDP_P1=1` (Linux, cleartext):

1. mio SO_REUSEPORT shards own accept (Tokio accept skipped for that bind).
2. MSG_PEEK eligibility: `GET /api/` → SDP; else handoff once to Hyper/Cap067.
3. Fresh `ProxyExecutionPlan` per request from `GenerationProjection` (ArcSwap).
4. Route via RuntimePlan SSOT; WAF via generation-frozen `WafEngine` in-shard.
5. Known-length response relay; client + upstream keepalive; shard-local upstream idle pool.

## Value gate (Netcup P4)

| Case | Condition |
|------|-----------|
| SP1-A | RPS ≥ +20% vs current ExyonQ (flag off) + correctness PASS |
| SP1-B | +15% ≤ gain < 20% → owner review |
| SP1-C | gain < 15% → STOP architecture integration |

## Harness

`scripts/remote/v044-sdp-p1-get-known-length.sh`

## Holds

PXDP / body relay / P6 / H2 / H3 / WS / CONNECT = HOLD
Cap067 / Tokio / Hyper role-scoped = PRESERVE
PUSH / MERGE / TAG / RELEASE = NO
