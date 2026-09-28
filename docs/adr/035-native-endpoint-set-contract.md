# ADR 035 — Native Endpoint Set Contract

```text
ADR = 035
TITLE = Native Endpoint Set Contract
STATUS = ACCEPTED
DATE = 2026-07-30
PHASE = K8S-P1
OWNER_ACCEPTANCE = ACCEPT
K8S_P1_STATUS = CLOSED_ACCEPTED
K8S_P1_CLOSED = YES
# Historical P1 documentary authorization — do not reinterpret as runtime WRR admit.
IMPLEMENTATION_AUTHORIZED = NO
IMPLEMENTATION_STATUS = PRODUCTIVE_WEIGHTED_MULTI_ENDPOINT_IMPLEMENTED_PENDING_LINUX_PROTECTORS
K8S_P2A_STATUS = CLOSED_ACCEPTED_FOUNDATION_RUNTIME_EXECUTION_DEFERRED
K8S_P2A_CLOSED = YES
K8S_P2A_IR_IMPLEMENTED = YES
K8S_P2A_RUNTIME_PLAN_IMPLEMENTED = YES
K8S_P2A_MULTI_ENDPOINT_EXECUTION_IMPLEMENTED = NO
K8S_P2B_OPENED = YES
K8S_P2B_STATUS = IMPLEMENTATION_COMPLETE_COMMITTED_PENDING_LINUX_PROTECTORS
K8S_P2B_CLOSED = NO
K8S_P2B_CODE_ACCEPTED_FOR_LOCAL_COMMIT = YES
K8S_P2B_PRODUCTIVE_RELEASE_ACCEPTED = NO
K8S_P2B_WRR_IMPLEMENTED = YES
K8S_P2B_PRIORITY_BANDS_IMPLEMENTED = YES
K8S_P2B_MULTI_ENDPOINT_EXECUTION_IMPLEMENTED = YES
P2B_OPEN_FINDING_ID = P2B-OPEN-001
P2B_OPEN_FINDING_BLOCKS_P2B_CLOSE = YES
P14V043_OPENED = NO
K8S_P2_OPENED = YES
K8S_P2A_OPENED = YES
K8S_P2C_OPENED = NO
K8S_P2C_READY_TO_ADMIT = NO_PENDING_P2B_LINUX_PROTECTORS
K8S_P3_OPENED = NO
EXYONQ_IMPLEMENTATION_LANGUAGE_POLICY = RUST_ONLY
EXYONQ_GO_CODE_ALLOWED = NO
EXYONQ_K8S_CONTROLLER_LANGUAGE_RECOMMENDATION = RUST_CONTROLLER_OVER_RUST_NATIVE_CONTRACT
EXYONQ_K8S_CONTROLLER_LANGUAGE_DECISION_STATUS = OWNER_DECIDED_RUST_ONLY
ACTIVE_GO_RECOMMENDATIONS_REMAINING = 0
P2A_OPEN_FINDING_ID = P2A-OPEN-001
P2A_OPEN_001_STATUS = CLOSED
P2A_OPEN_FINDING_BLOCKS_P2B_CLOSE = NO
```

P2B note: productive smooth WRR + priority bands are **conditionally accepted** for a **local provisional commit** only (`OWNER_IMPLEMENTATION_ACCEPTANCE=CONDITIONAL_ACCEPT_PENDING_LINUX_PERFORMANCE_PROTECTORS`). K8S-P2B is **not closed**. Finding **P2B-OPEN-001** (Linux performance/contention protectors on Netcup amd64 + Oracle arm64) blocks P2B close, P2C admit, and release claims. `IMPLEMENTATION_AUTHORIZED=NO` retains the historical P1 documentary gate (controller still not authorized by ADR acceptance alone).

## CONTEXT

ExyonQ upstreams are single-target (`UpstreamConfig { name, target, timeout_ms }` in `config/ir/src/lib.rs`). Discovery overlays may list multiple endpoints but `apply_discovery` keeps only `endpoints.first()` (`config/merge/src/lib.rs`). Kubernetes, panels, Nexus, and multi-node all need a shared multi-endpoint model that is **not** EndpointSlice-shaped.

## DECISION

Adopt an explicit native **EndpointSet** keyed by **BackendId**, with stable **EndpointId**s, weights, priority bands, admin enable/disable/drain-request flags, and **no dynamic health fields in Config IR**.

```text
EXYONQ_NATIVE_ENDPOINT_SET_MODEL = EXPLICIT_ENDPOINT_SET_WITH_STABLE_IDS
EXYONQ_NATIVE_BACKEND_IDENTITY_MODEL = HYBRID_USER_ID_PLUS_FINGERPRINT_PLUS_SOURCE
EXYONQ_NATIVE_LB_MVP = WEIGHTED_ROUND_ROBIN_WITH_PRIORITY_BANDS
EXYONQ_NATIVE_HEALTH_STATE_MODEL = DESIRED_ADMIN_IN_IR_PLUS_OBSERVED_RUNTIME_STATE
EXYONQ_NATIVE_DRAIN_MODEL = BEST_EFFORT_ADMIN_DRAIN_WITH_DEADLINE
EXYONQ_NATIVE_ROUTE_BACKEND_REFERENCE_MODEL = ROUTE_TO_BACKEND_ID_TO_ENDPOINT_SET
```

## ALTERNATIVES

1. Keep single `target` forever — rejected (blocks all niches needing LB).
2. Encode Kubernetes EndpointSlice in IR — rejected (violates K8s-agnostic rule).
3. Per-route embedded endpoint lists — rejected (duplication; poor identity).
4. Opaque blob from controller — rejected (not reusable by panels/native).
5. EndpointSet + BackendId as designed — **selected**.

## DATA_MODEL

Conceptual (not authorized to implement):

- `EndpointSet { backend_id, generation, policy, endpoints: Vec<Endpoint> }`
- `Endpoint { endpoint_id, address, port, protocol, weight, priority, admin_state, metadata, source_id? }`
- Legacy `target` sugar expands to one endpoint.

## IDENTITY

- BackendId user-provided, stable, collision → reject.
- EndpointId producer-stable preferred; else derived from normalized address/port/protocol.
- No mandatory namespace/service/pod/EndpointSlice UID.

## LOAD_BALANCING

First implementation: weighted round-robin within priority bands. Equal weights ⇒ RR. weight 0 / disabled / draining / unhealthy excluded per rules in native-contracts doc.

## HEALTH_SEPARATION

IR holds desired admin state only. Runtime holds ready/draining/unhealthy/ejected/unknown. Adapter readiness is a hint, not the native model.

## DRAINING

Best-effort with mandatory deadline. Current process-level drain is insufficient for per-endpoint promises; P2 must implement accounting honestly or narrow claims.

## GENERATIONS

EndpointSet updates travel inside immutable snapshots. Full replace per backend in MVP snapshot. Picker rebuilt off request path.

## LIMITS

Provisional defaults/hard max documented in Contract 15 of `k8s-p1-native-contracts.md`. Empty selectable set ⇒ structured runtime error.

## SECURITY

Metadata must not carry private keys. Addresses may appear in logs carefully; metrics use bounded ids, not raw pod IP labels.

## OBSERVABILITY

BackendId/EndpointId stable; metric tokens bounded; `RAW_POD_IP_AS_UNBOUNDED_METRIC_LABEL = FORBIDDEN`.

## COMPATIBILITY

Accept legacy single `target` as sugar. Discovery multi-endpoint JSON must map to full EndpointSet (stop first-only collapse) in P2.

## CONSEQUENCES

- Enables K8s, panels, and native multi-node to share one DP feature.
- Requires RuntimePlan/proxy changes in P2.
- Schema prefers additive sugar then deprecation.

## REJECTED_OPTIONS

- Native EndpointSlice types
- Health-in-IR as persistent config
- Snippet-based upstream blocks

## OPEN_QUESTIONS

1. Exact hard max endpoints (needs memory evidence).
2. Whether route-level multi-backend weights ship with first IR or only endpoint weights.
3. How much per-endpoint drain accounting is feasible in P2 vs process drain only.

```text
STATUS = ACCEPTED
IMPLEMENTATION_AUTHORIZED = NO
# ADR accept ≠ product authorize. P2A foundation was separately admitted:
K8S_P2A_FOUNDATION_AUTHORIZED = YES
K8S_P2A_WRR_RUNTIME_AUTHORIZED = NO
K8S_P2B_OPENED = NO
K8S_P1_STATUS = CLOSED_ACCEPTED
K8S_P1_CLOSED = YES
OWNER_ACCEPTANCE = ACCEPT
```
