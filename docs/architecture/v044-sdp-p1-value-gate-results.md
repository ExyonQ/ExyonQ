# V044 SDP-P1 performance / value gate

```text
WIP = V044_SPECIALIZED_PROXY_DATAPLANE_SDP_P1_GET_KNOWN_LENGTH
CASE = SP1-C
HOST = Netcup amd64 cpuset 0-7
REPS = 5 interleaved
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
EVIDENCE = .exyonq-local/evidence/sdp-p1/20260827-210315/
PRODUCT_BINARY_SHA256 = efb15c7a7ce459c403fc837f3cf125f9505c5ccfdff93c819136936220964e00
```

## Medians (fresh same campaign)

| Product | RPS | CPU µs/req | RSS KiB |
|---------|----:|-----------:|--------:|
| A0 ExyonQ (SDP off) | 79081 | 35.38 | 28304 |
| A1 ExyonQ SDP-P1 | 81405 | 34.19 | 27924 |
| NGINX | 96194 | 26.71 | 6064 |
| OLS | 103754 | 22.28 | 3608 |
| HAProxy | 33126 | 40.73 | 18672 |

```text
SDP_P1_VS_CURRENT_EXYONQ_RPS_PERCENT = +2.94%
GATE_MINIMUM = 15%
GATE_PREFERRED = 20%
INTEGRATION_VALUE_GATE = FAIL
```

## Correctness

```text
CORRECTNESS = PASS (single GET, keepalive 200, TE reject)
SDP_P1_EXECUTION_CONFIRMED = YES (boot: sdp-p1 enabled, shards=4)
LATENCY_MEASUREMENT_VALID = NO (rewrk JSON p50/p95/p99 null this harness)
```

## Economic kill switch

```text
SDP_P1_RPS_GAIN_VS_CURRENT_EXYONQ < 15%
  => STOP_ARCHITECTURE_INTEGRATION
PRODUCT_IMPLEMENTATION_CONTINUE_RECOMMENDATION = NO
NEXT_PRODUCT_WIP_PROPOSAL = NONE
```

EP-C prototype +38% did **not** survive product integration at the required margin.
Do not stack SDP-P2+ features hoping to recover RPS.

Post-mortem (read-only): [`v044-sdp-p1-integration-loss-attribution.md`](v044-sdp-p1-integration-loss-attribution.md) — **CASE=IL-F** (CLOSE specialized product integration; Cap067-class user CPU restored; +2.94% optimistic vs L1–L4-complete path).
