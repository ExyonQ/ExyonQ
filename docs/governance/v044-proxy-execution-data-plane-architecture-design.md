# V044 — Proxy execution data plane architecture design (governance index)

```text
STATUS = COMPLETE
CASE = PXDP-B
WIP = V044_PROXY_EXECUTION_DATA_PLANE_ARCHITECTURE_DESIGN
MODE = READ_ONLY_DESIGN_AND_CONTRACT
PRODUCT_MUTATION = NO
PROTOTYPE = NO
PARENT_CASE = DP-C
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

SOURCE_HEAD = ade997a362e60d1dc6eafc984da454930600845a
SOURCE_TREE = 4ae6ffc7528d0881be23fcac246c345dcb9a1606
SOURCE_BINARY_SHA256 = e28c0abe18587f2d911d492be64cbec42372383e011417a6c578be52da5ba459
```

**Canonical design (full detail, diagrams, tables):**

[`../architecture/v044-proxy-execution-data-plane-architecture-design.md`](../architecture/v044-proxy-execution-data-plane-architecture-design.md)

---

## Executive verdict

```text
V044_PROXY_EXECUTION_DATA_PLANE_ARCHITECTURE_DESIGN_STATUS = COMPLETE
CASE = PXDP-B

RECOMMENDED_DESIGN =
  Dedicated worker-local ProxyConnection execution shell (E2+E3):
  one connection task, frozen ProxyRequestPlan, UpstreamLease, ProxyBodyRelay
  (Incoming → DownstreamWriter without BoxBody), Hyper as upstream protocol engine only.

P3_ARCHITECTURE_PRESERVED = YES
CAP034_IN_TARGET_ARCHITECTURE = EVOLVE_AS_PART_OF_NEW_LIFECYCLE
BODY_PIPELINE_EXPERIMENT = HOLD / PARTIAL_REUSE ideas only

EXPECTED_RPS_GAIN = PLAUSIBLE_NOT_PROVEN
MINIMUM_ACCEPTABLE_FUTURE_RPS_GAIN = >= 12%
MINIMUM_ACCEPTABLE_FUTURE_CPU_GAIN = >= 15%

NEXT_PRODUCT_WIP_PROPOSAL = NONE
NEXT_DESIGN_WIP_PROPOSAL = V044_PROXY_EXECUTION_DATA_PLANE_SHADOW_PLAN (PXDP-P1 — owner review; not started)
NEXT_MEASUREMENT_WIP_PROPOSAL = NONE

SHOULD_BODY_PIPELINE_EXPERIMENT_RESUME = NO
V044_P4_PROXY_WIRE_BODY_PIPELINE_SLIM = HOLD
```

## Strategic supersession note (GA-B → EP-C → SD-B)

```text
PXDP_FUTURE = HOLD (execution path)
PXDP_P6 = HOLD
REQUEST_BODY_RELAY = HOLD
P4_MICRO_OPTIMIZATION_CAMPAIGN = STOPPED
EP-C = ACCEPTED (mio TPC geometry)
SD-B = COMPLETE (contracts; SDP-P1 NOT AUTHORIZED)
PXDP_P0_TESTS = PRESERVE_ADAPT as SDP contract oracles
PXDP_HYPER_EXECUTION = RETIRE_AFTER_SDP_SUCCESS (not now)
```

See [`v044-specialized-proxy-dataplane-integration-design.md`](v044-specialized-proxy-dataplane-integration-design.md).

---

## Count summary (current → target)

| Metric | Current (bounded) | Target |
| --- | --- | --- |
| Stages | 19 | **8** |
| Task boundaries / req | 4–6 | **≤2** |
| Channel boundaries / req | 2–4 | **≤1** |
| Thread handoffs / req | 1–2 | **≤1** |
| Body type transitions | 8–12 | **2–3** |
| Shared state touches | 4–6 | **1–2** |

---

## Contracts (PXDP-P0 — complete)

- [v044-proxy-execution-data-plane-contracts-ir.md](v044-proxy-execution-data-plane-contracts-ir.md) — **CASE P0-A**; frozen IR/contracts; no product code

## Migration phases (design only — P0 complete; implementation not started)

`PXDP-P0` contracts ✓ → `P1` shadow plan → `P2` narrow H1 shell → `P3` relay/backpressure →
`P4` framing/keepalive → `P5` policy integration → `P6` adversarial correctness →
`P7` isolated A/B → `P8` dual Linux → `P9` productization + legacy retirement.

---

## Related

- [v044-data-plane-architecture-strategic-review.md](v044-data-plane-architecture-strategic-review.md)
- [v044-p4-proxy-execution-tax-decomposition.md](v044-p4-proxy-execution-tax-decomposition.md)
- [v044-p4-proxy-wire-body-pipeline-slim.md](v044-p4-proxy-wire-body-pipeline-slim.md)

```text
NOTION_UPDATE_REQUIRED = YES
DOCS_RECONCILED = YES
PUSH = NO
```
