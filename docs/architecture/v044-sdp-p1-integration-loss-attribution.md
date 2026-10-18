# V044 SDP-P1 integration loss attribution

```text
WIP = V044_SDP_P1_INTEGRATION_LOSS_ATTRIBUTION
PARENT = SDP-P1 / SP1-C
CASE = IL-F
SECONDARY = IL-B, IL-C
MODE = READ_ONLY
PRODUCT_MUTATION = NO
EVIDENCE = .exyonq-local/evidence/sdp-p1-integration-loss/20260827-220348/
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
```

Governance: [`../governance/v044-sdp-p1-integration-loss-attribution.md`](../governance/v044-sdp-p1-integration-loss-attribution.md)
Parents: [`v044-sdp-p1-value-gate-results.md`](v044-sdp-p1-value-gate-results.md), [`v044-specialized-proxy-event-loop-control.md`](v044-specialized-proxy-event-loop-control.md)

---

## Primary answer

```text
WHERE_DID_THE_EP_C_PLUS_38_PERCENT_GO =
  The EP-C advantage was almost entirely near-zero user-space CPU
  (1.77 µs/req) in a bare mio process. SDP-P1 re-embedded the same
  geometry inside the full ExyonQ Cap067/Tokio process, restoring
  Cap067-class user CPU (18.82 µs/req, +17.05 vs EP-C). System CPU
  improved slightly (−4.46). Net +12.53 µs/req → RPS fell from 97.5k
  to 81.4k (−16.5% vs EP-C) while beating A0 by only +2.94%.
```

Outcome: **IL-F** (no credible path to competitive target), with **IL-B** (distributed real product cost) and **IL-C** (embedding boundary: specialized loop inside Cap067 process). Not **IL-A** (no one/two proven ≥15% repair roots).

---

## Comparability

Cross-campaign absolute RPS is only **PARTIAL** (ExyonQ floor and OLS drifted; EP-C ExyonQ compose had PXDP env, SDP A0 did not).
**User/system CPU pattern** and within-campaign A0 vs A1 are **STRONG**.

| DIMENSION | EP-C | SDP-P1 | PERF | CORRECTNESS |
|-----------|------|--------|------|-------------|
| binary | bare specialized-proxy | full exyonq | HIGH | HIGH |
| mio / httparse | 1.x / 1.10.1 | 1.x / 1.10.1 | LOW | LOW |
| socket2 | 0.5.10 | 0.6.5 | LOW | LOW |
| host / cpuset | Netcup 0-7 | Netcup 0-7 | HIGH | — |
| shards | 4 | 4 | HIGH | — |
| Tokio co-resident | no | yes (workers + notify + control) | CRITICAL | MED |
| WAF | none | under-inspect (L4) | HIGH | CRITICAL |
| routing / generation | hardcoded | RuntimePlan + ArcSwap | HIGH | HIGH |
| headers | minimal | incomplete L1 | HIGH | CRITICAL |
| user µs/req | **1.77** | **18.82** | CRITICAL | — |
| system µs/req | 19.84 | 15.38 | HIGH | — |
| RPS | 97509 | 81405 | CRITICAL | — |
| RSS KiB | ~3652 | ~27924 | HIGH | — |

---

## CPU attribution (campaign medians)

| | EP-C proto | SDP-P1 | Δ |
|--|----------:|-------:|--:|
| user µs/req | 1.77 | 18.82 | **+17.05** |
| system µs/req | 19.84 | 15.38 | −4.46 |
| total µs/req | 21.66 | 34.19 | **+12.53** |

```text
ATTRIBUTED_CPU_US_PER_REQ = 12.53   # net = measured user+system deltas
UNATTRIBUTED_CPU_US_PER_REQ = ~0 for net
SUBROOT_SPLIT_OF_USER = NOT_MEASURED
ATTRIBUTION_COVERAGE_PERCENT ≈ 100 of net
```

Live same-stack 1-rep confirmed user/system pattern; absolute RPS contaminated by host cohabitation — not used for ranking.

---

## Feature / root summary

Dominant: **ROOT-19** Cap067/Tokio process residency + distributed **REQUIRED_PRODUCT** edges (plan, routing, generation, incomplete WAF/headers).
Syscall geometry is **not** the loss (SDP system CPU better than EP-C).

```text
CURRENT_BENCHMARK_OPTIMISTIC_DUE_TO_OPEN_DEFECTS = YES
  L1/L4 incomplete work → +2.94% is an optimistic upper bound
WOULD_CORRECT_FIX_LIKELY = REDUCE performance
```

---

## Competitive headroom

```text
OLS = 103754
PLUS25_TARGET ≈ 129693
SDP_P1_TO_TARGET_REQUIRED_GAIN ≈ +59.32%
CURRENT_P4_CPU_COMPETITIVE_TARGET = 22.28 us/req (OLS, same SDP campaign)
CREDIBLE_RECOVERY_TO_15_PERCENT_INTEGRATION_GATE = NO
CREDIBLE_RECOVERY_TO_FINAL_COMPETITIVE_TARGET = NO
IS_56_CELL_PRODUCTION_ADMISSION_ECONOMICALLY_JUSTIFIED_YET = NO
IS_SDP_P1_COMPLEXITY_JUSTIFIED_FOR_PLUS_2_94_PERCENT = NO
NET_ENGINEERING_VALUE = NEGATIVE
```

EP-C was OLS-class, not +25% OLS. Do not erase EP-C; do not promote it to product evidence.

```text
P95_TARGET_PROGRESS = UNKNOWN
P99_TARGET_PROGRESS = UNKNOWN
LATENCY_MEASUREMENT_VALID = NO
```

---

## Decision

```text
SPECIALIZED_PRODUCT_INTEGRATION_DIRECTION = CLOSE
GA_B_GLOBAL_DECISION = UNCHANGED / PROXY_PRODUCTIZATION_FAILED
NEXT_PRODUCT_REPAIR_PROPOSAL = NONE
NEXT_STRATEGIC_PROPOSAL = NONE
SDP_P2_PLUS = BLOCKED
EXYONQ_SDP_P1 = DEFAULT_OFF
EXECUTION_GEOMETRY_VALUE = PROVEN (EP-C)
PRODUCT_INTEGRATION_VALUE = FAILED_CURRENT_BOUNDARY
```
