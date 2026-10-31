# V044 — Global execution architecture strategic review (governance)

```text
WIP = V044_GLOBAL_EXECUTION_ARCHITECTURE_STRATEGIC_REVIEW
STATUS = COMPLETE
CASE = GA-B
MODE = READ_ONLY_WHOLE_PRODUCT_EXECUTION_ARCHITECTURE_REVIEW
PRODUCT_MUTATION = NO
ARCHITECTURE_MUTATION = NO
HARNESS_MUTATION = NO
CANONICAL_MERGE = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

SOURCE_HEAD = 0bc2b973e5ff62b2316fbc7c3f002d673c65d791
SOURCE_TREE = b1b7659b4d3bb8745ea0ed42a8de7d424f523767
```

Architecture: [`../architecture/v044-global-execution-architecture-strategic-review.md`](../architecture/v044-global-execution-architecture-strategic-review.md)  
Performance: [`../performance/v044-global-execution-architecture-strategic-review.md`](../performance/v044-global-execution-architecture-strategic-review.md)  
Evidence: `.exyonq-local/evidence/global-execution-architecture-review/20260827-181625/`

Parents: PR-C · RC-B · UG-C · SW-C · MC-B · WG-B/EC-A · RT-C · CP-C · DP-C · PXDP-B

Orchestrator admission: **ACCEPT** (read-only). Tech-lead CASE draft: **GA-B**. Whole-product rewrite: **REJECT_ARCHITECTURE**.

---

## Verdict

| Field | Value |
| --- | --- |
| CASE | **GA-B** — hybrid execution architecture recommended |
| Whole-product rewrite | **NO** |
| Product logic rewrite | **NO** |
| Execution substrate change | **PARTIAL** (proxy dataplane) |
| Hyper replacement | **PARTIAL** (proxy hot path role) |
| Tokio replacement | **NO** |
| Preserve Cap067 | **YES** |
| Preserve routing / WAF / module API / observability / reload | **YES** |
| PXDP future | **HOLD** |
| P4 micro-opt campaign | **STOPPED** |

---

## Strategic HOLDs (authoritative until owner reopens)

```text
REQUEST_BODY_RELAY = HOLD
PXDP_P6 = HOLD
PXDP_FEATURE_EXPANSION = HOLD
H2_H3_EXPANSION = HOLD_IF_EXECUTION_DEPENDENT
WEBSOCKET_CONNECT_ON_CURRENT_PROXY_SHELL = HOLD
MULTI_CONCURRENCY_SWEEP = HOLD
WORKER_DEFAULT_CHANGE = NOT_AUTHORIZED
CAP067_REMOVAL = NOT_AUTHORIZED
P4_MICRO_OPTIMIZATION_CAMPAIGN = STOPPED
```

Do **not** continue building proxy features on an execution geometry that may be replaced.

---

## Recommended next (separate admission)

```text
NEXT_PROTOTYPE_PROPOSAL =
  isolated non-product mio / thread-per-core event-loop H1 reverse proxy
EXPECTED_VALUE_GATE = >=15% median P4 RPS vs ExyonQ PXDP
PRODUCT_MUTATION = still NO until prototype clears gate + owner auth
```

This governance document does **not** authorize implementing the prototype.

---

## Authority held

```text
PRODUCT_MUTATION = NO
ARCHITECTURE_MUTATION = NO
CANONICAL_MERGE = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION = NO
```

## Follow-on prototype terminal

```text
V044_SPECIALIZED_PROXY_EVENT_LOOP_CONTROL = COMPLETE
CASE = EP-C
SPECIALIZED_PROXY_DATAPLANE_JUSTIFIED = YES
```

## Follow-on integration design terminal

```text
V044_SPECIALIZED_PROXY_DATAPLANE_CONTRACT_AND_INTEGRATION_DESIGN = COMPLETE
CASE = SD-B
NEXT_PRODUCT_WIP_PROPOSAL = V044_SPECIALIZED_PROXY_DATAPLANE_SDP_P1_GET_KNOWN_LENGTH
PRODUCT_IMPLEMENTATION_AUTHORIZED = NO
SoT = v044-specialized-proxy-dataplane-integration-design.md
  / ../architecture/v044-specialized-proxy-dataplane-contract-integration-design.md
```

## SDP-P1 / attribution terminal (later)

```text
SDP_P1 = COMPLETE / SP1-C (integration value FAIL)
ATTRIBUTION = IL-F → SPECIALIZED_PRODUCT_INTEGRATION_DIRECTION=CLOSE
GA_B = UNCHANGED / PROXY_PRODUCTIZATION_FAILED
SoT = v044-sdp-p1-integration-loss-attribution.md
```

## SPPG terminal (final architecture kill test)

```text
SPPG = COMPLETE
CASE = SPPG-C (INSUFFICIENT_COMPETITIVE_HEADROOM)
MINIMUM_CONTINUATION_PASS = NO
EXTREME_OBJECTIVE_DISPOSITION = STOP_PURSUING
NEXT_ARCHITECTURE_PROTOTYPE = NONE
SoT = v044-split-plane-near-product-proving-ground.md
```

## Competitive Frontier Phase 0 (post-SPPG strategy reset)

```text
V044_CORE_ARCHITECTURE_RECONCILIATION_FOR_COMPETITIVE_FRONTIER = COMPLETE
CASE = CFR-B
TARGET_PROCESS_MODEL = HYBRID (owner confirmation)
PHASE_1_AUTHORIZED = NO
SoT = v044-core-architecture-reconciliation-competitive-frontier.md
```

Extreme +25/−10 remains CLOSED. Owner Competitive Frontier target ≈ SPPG deltas vs OLS (architecture reference only).
