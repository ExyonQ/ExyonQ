# V044 — Specialized proxy event-loop control (governance)

```text
WIP = V044_SPECIALIZED_PROXY_EVENT_LOOP_CONTROL
STATUS = COMPLETE
CASE = EP-C
PARENT_CASE = GA-B
MODE = ISOLATED_NON_PRODUCT_ARCHITECTURE_PROTOTYPE
PRODUCT_MUTATION = NO
EXYONQ_PRODUCT_CODE_MUTATION = NO
CONTROL_IS_PRODUCT = NO
CANONICAL_MERGE = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
```

Architecture: [`../architecture/v044-specialized-proxy-event-loop-control.md`](../architecture/v044-specialized-proxy-event-loop-control.md)  
Performance: [`../performance/v044-specialized-proxy-event-loop-control.md`](../performance/v044-specialized-proxy-event-loop-control.md)  
Parent: [`v044-global-execution-architecture-strategic-review.md`](v044-global-execution-architecture-strategic-review.md)

---

## Verdict

| Field | Value |
| --- | --- |
| CASE | **EP-C** — competitive breakthrough vs NGINX/OLS class on P4 |
| Value gate ≥15% vs ExyonQ | **PASS** (+38.2%) |
| Specialized dataplane justified | **YES** |
| Whole-product rewrite | **NO** |
| Product logic rewrite | **NO** |
| Tokio global | **PRESERVE** |
| Hyper global | **ROLE_SCOPED** |
| Cap067 | **PRESERVE** (untouched) |
| PXDP / body relay / P6 | **HOLD** |

---

## Next (value gate FAIL — stop)

```text
V044_SPECIALIZED_PROXY_DATAPLANE_SDP_P1_GET_KNOWN_LENGTH = COMPLETE
CASE = SP1-C
INTEGRATION_VALUE_GATE = FAIL
STOP_ARCHITECTURE_INTEGRATION = YES
NEXT_PRODUCT_WIP_PROPOSAL = NONE
SoT = docs/governance/v044-sdp-p1-get-known-length.md
```

Attribution (read-only): [`v044-sdp-p1-integration-loss-attribution.md`](v044-sdp-p1-integration-loss-attribution.md) — **IL-F** CLOSE specialized product integration. EP-C geometry remains valid non-product control.
---

## Authority held

```text
PRODUCT_MUTATION = NO
CANONICAL_MERGE = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION = NO
```
