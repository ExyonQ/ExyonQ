# V044 — Proxy execution data plane contracts & IR (governance index)

```text
STATUS = COMPLETE
CASE = P0-A
WIP = V044_PROXY_EXECUTION_DATA_PLANE_CONTRACTS_IR
PHASE = PXDP-P0
MODE = READ_ONLY_CONTRACTS_AND_INTERMEDIATE_REPRESENTATION_DESIGN
PARENT_WIP = V044_PROXY_EXECUTION_DATA_PLANE_ARCHITECTURE_DESIGN
PARENT_CASE = PXDP-B
PRODUCT_MUTATION = NO
PROTOTYPE = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

SOURCE_HEAD = ade997a362e60d1dc6eafc984da454930600845a
SOURCE_TREE = 4ae6ffc7528d0881be23fcac246c345dcb9a1606
SOURCE_BINARY_SHA256 = e28c0abe18587f2d911d492be64cbec42372383e011417a6c578be52da5ba459
```

**Canonical contracts document (full detail, diagrams, tables):**

[`../architecture/v044-proxy-execution-data-plane-contracts-ir.md`](../architecture/v044-proxy-execution-data-plane-contracts-ir.md)

---

## Strategic supersession (EP-C / SD-B)

```text
PXDP_P0_CONTRACT_SEMANTICS = PRESERVE_ADAPT → ProxyExecutionPlan (SDP)
PXDP_HYPER_EXECUTION = HOLD / RETIRE_AFTER_SDP_SUCCESS
PXDP_P6 = HOLD
REQUEST_BODY_RELAY = HOLD
SD-B = docs/governance/v044-specialized-proxy-dataplane-integration-design.md
```

---

## Executive verdict

```text
V044_PROXY_EXECUTION_DATA_PLANE_CONTRACTS_IR_STATUS = COMPLETE
CASE = P0-A

CONTRACTS_FROZEN =
  ProxyRequestPlan (REQUEST-only lifetime; replan every keepalive request)
  ProxyConnection + ProxyPhase state machines
  UpstreamLease (RAII; global Hyper pool v1 boundary)
  ProxyBodyRelay + DownstreamWriter + FramingDecision + ConnectionDisposition
  ProxyError SSOT + RetryPolicyView + ProxyTimers + ProxyObsSpan
  WAF/routing/generation/hop-by-hop handoffs preserved from source truth

KEY_INVARIANTS =
  MAX_ACTIVE_H1_REQUESTS_PER_CONNECTION = 1
  H1_PIPELINING = REJECTED v1
  RETRY_AFTER_COMMIT = NO
  UPSTREAM_BEFORE_WAF_ALLOW = FORBIDDEN
  WORKER_LOCAL = task ownership (NOT OS-thread pinning)
  PXDP_P1_SHADOW = DECISION_SHADOW_ONLY

SAFE_RUST_OWNERSHIP_FEASIBLE = YES
SELF_REFERENTIAL_UNSAFE_REQUIRED = NO

NEXT_WIP_PROPOSAL = V044_PROXY_EXECUTION_DATA_PLANE_SHADOW_PLAN
NEXT_PHASE = PXDP-P1 (NOT AUTHORIZED)
PXDP_P1_IMPLEMENTATION_AUTHORIZED = NO
```

---

## Migration sequence (unchanged — not started)

`PXDP-P0` contracts ✓ → `PXDP-P1` shadow plan → `P2` narrow H1 shell → …

---

## Related

- [v044-proxy-execution-data-plane-architecture-design.md](v044-proxy-execution-data-plane-architecture-design.md)
- [v044-data-plane-architecture-strategic-review.md](v044-data-plane-architecture-strategic-review.md)
- [v044-cap034-proxy-framing-keepalive-contract-review.md](v044-cap034-proxy-framing-keepalive-contract-review.md)

```text
NOTION_UPDATE_REQUIRED = YES
DOCS_RECONCILED = YES
PUSH = NO
```
