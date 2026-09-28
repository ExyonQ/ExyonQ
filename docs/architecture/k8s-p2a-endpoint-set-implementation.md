# K8S-P2A — Config IR Compatibility and Endpoint-Set Foundation

```text
DOCUMENT = docs/architecture/k8s-p2a-endpoint-set-implementation.md
PHASE = K8S-P2A
OWNER_ACCEPTANCE = ACCEPT
K8S_P2A_STATUS = CLOSED_ACCEPTED_FOUNDATION_RUNTIME_EXECUTION_DEFERRED
K8S_P2A_CLOSED = YES
K8S_P2A_IR_IMPLEMENTED = YES
K8S_P2A_RUNTIME_PLAN_IMPLEMENTED = YES
K8S_P2A_MULTI_ENDPOINT_EXECUTION_IMPLEMENTED = NO
K8S_P2A_WRR_RUNTIME_IMPLEMENTATION = DEFER_TO_K8S_P2B
K8S_P2A_PRODUCT_CLAIM_MULTI_ENDPOINT = FORBIDDEN
K8S_P2A_PRODUCT_CLAIM_LOAD_BALANCING = FORBIDDEN
K8S_P2_OPENED = YES
K8S_P2A_OPENED = YES
K8S_P2B_OPENED = YES
K8S_P2B_STATUS = IMPLEMENTATION_COMPLETE_COMMITTED_PENDING_LINUX_PROTECTORS
K8S_P2B_CLOSED = NO
K8S_P2B_READY_TO_ADMIT = ADMITTED_WRR_EXECUTION_PROVISIONAL
P2B_OPEN_FINDING_ID = P2B-OPEN-001
P2B_OPEN_FINDING_BLOCKS_P2B_CLOSE = YES
K8S_P2C_OPENED = NO
K8S_P2C_READY_TO_ADMIT = NO_PENDING_P2B_LINUX_PROTECTORS
K8S_P3_OPENED = NO
K8S_P3_READY_TO_ADMIT = NO_PENDING_K8S_P2_AND_RUST_CONTROLLER_SPIKES
K8S_P6_OPENED = NO
K8S_P6_READY_TO_ADMIT = YES_FOR_SEPARATE_SCOPE
P14V043_OPENED = NO
COMMIT_AUTHORIZED = YES
PUSH_AUTHORIZED = NO
EXYONQ_IMPLEMENTATION_LANGUAGE_POLICY = RUST_ONLY
ACTIVE_GO_RECOMMENDATIONS_REMAINING = 0
KUBERNETES_TYPES_IN_SHARED_CONTRACTS = 0
P2A_OPEN_001_STATUS = CLOSED
```

## Scope

Shared Rust data-plane foundation only. Not a Kubernetes controller.

## Decisions implemented

```text
BACKEND_ID_MODEL = USER_BACKEND_ID_STRING_NEWTYPE (IR)
  # Distinct from dense RuntimePlan BackendId(u32) ADR-029
ENDPOINT_ID_MODEL = EXPLICIT_OR_DETERMINISTIC_HEX
ENDPOINT_SET_MODEL = EXPLICIT_ENDPOINT_SET_WITH_STABLE_IDS
SELECTION_POLICY_MODEL = EndpointSelectionPolicy::WeightedRoundRobin
FAILOVER_POLICY_MODEL = EndpointFailoverPolicy::PriorityBands | None
# Policies remain separate enums — no combined rigid enum.
LOAD_BALANCING_SELECTION_POLICY = WEIGHTED_ROUND_ROBIN
FAILOVER_PRIORITY_POLICY = PRIORITY_BANDS_OR_NONE
EMPTY_ENDPOINT_SET_SEMANTICS = CONFIG_VALID_BUT_BACKEND_UNAVAILABLE
TARGET_AND_ENDPOINTS_CONFLICT = REJECT_AMBIGUOUS_CONFIGURATION
LEGACY_TARGET_COMPATIBILITY = ACCEPT_AND_NORMALIZE_TO_SINGLE_ENDPOINT_SET
SILENT_ENDPOINT_TRUNCATION = NO
FIRST_ENDPOINT_FALLBACK = FORBIDDEN
MULTI_ENDPOINT_FALSE_ADVERTISEMENT = FORBIDDEN
P2A_WRR_RUNTIME_IMPLEMENTATION = DEFER_TO_K8S_P2B
HOSTNAME_RESOLUTION = PRESERVE_IN_URI_LEGACY_BEHAVIOR (no new DNS at compile)
RUNTIME_PLAN_ENDPOINT_REPRESENTATION = FULL_SET_PLUS_EXECUTION_GATE
K8S_P2A_SCHEMA_VERSIONING = ADDITIVE_SUGAR
```

## Identity

```text
ENDPOINT_ID_DERIVATION_FIELDS =
  address (EndpointAddress as configured: IPv4 | IPv6 | Hostname)
  port
  weight
  priority

ENDPOINT_ID_RUNTIME_VOLATILE_FIELDS = NONE
  # Explicitly excluded: health, readiness, connections, counters,
  # timestamps, generation, incidental order, resolved IP for hostnames.

ENDPOINT_ID_FORMAT = ep-{hex16}   # 64-bit DefaultHasher digest
ENDPOINT_ID_COLLISION_RISK = BIRTHDAY_64BIT
ENDPOINT_ID_COLLISION_BEHAVIOR = REJECT_DUPLICATE_OR_COLLISION
ENDPOINT_ID_COLLISION_TEST = PASS

USER_BACKEND_ID = contractual/configurable stable string (charset [A-Za-z0-9._-], max 128)
DENSE_BACKEND_ID = BackendId(u32) compiled index in RuntimePlan (ADR-029)
USER_TO_DENSE_BACKEND_MAPPING = DETERMINISTIC_WITHIN_SNAPSHOT
  # Sorted upstream names → dense cluster_id / BackendId within one compile.
DENSE_BACKEND_ID_STABILITY_ACROSS_GENERATIONS = NOT_GUARANTEED_UNLESS_EXPLICITLY_PROVEN
# Do not use dense BackendId as persistent identity in logs/metrics/control API/adapters.
```

## Empty endpoint set

```text
CONFIG_VALIDITY = VALID
BACKEND_RUNTIME_AVAILABILITY = UNAVAILABLE
EndpointExecutionStatus = EmptyUnavailable
# Does not invalidate the whole snapshot by itself.
# Route → empty backend → fail-closed (no panic, no accidental fallback).
# Test: route_to_empty_backend_is_unavailable_without_global_config_rejection
```

## Limits (provisional)

```text
MAX_ENDPOINTS_PER_BACKEND_PROVISIONAL = 256
MAX_BACKENDS_PROVISIONAL = 4096
MAX_ID_LENGTH_PROVISIONAL = 128
WEIGHT_RANGE = 0..=u32::MAX
PRIORITY_RANGE = 0..=u32::MAX
PORT = 1..=65535 (0 rejected)
```

## RuntimePlan execution gate

`CompiledCluster` retains full `endpoints[]` plus:

```text
EndpointExecutionStatus =
  SingleEndpointReady
  | EmptyUnavailable
  | MultiEndpointReady
```

```text
ENDPOINT_EXECUTION_STATUS (N>1) = MULTI_ENDPOINT_READY_PENDING_LINUX_PROTECTORS
```

K8S-P2B implements productive WRR (`ProxyCompiledSlot.multi_endpoint_executable`).
Phase not closed: **P2B-OPEN-001** (Linux protectors). See `k8s-p2b-wrr-execution.md`.

## Finding P2A-OPEN-001

```text
P2A-OPEN-001
TITLE = EndpointSet parser, normalization and deterministic identity require property/fuzz coverage before productive multi-endpoint execution.
STATUS = CLOSED
SEVERITY = MEDIUM
BLOCKS_P2A_CLOSE = NO
BLOCKS_P2B_CLOSE = NO
P2A_OPEN_001_EVIDENCE = config/ir/src/endpoint_set_property_tests.rs
CLOSED_BY = K8S-P2B
```

## Serialization policy

```text
DESERIALIZATION = LEGACY_TARGET_AND_NEW_ENDPOINTS_ACCEPTED
INTERNAL_NORMALIZATION = ENDPOINT_SET_ONLY
CANONICAL_FINGERPRINT_SINGLE = LEGACY_SHAPE (name/target/timeout_ms)
CANONICAL_FINGERPRINT_MULTI_OR_EMPTY = ENDPOINTS_PLUS_POLICIES
TOML_RENDER_SINGLE = PREFER_TARGET_SUGAR
SERDE_SCHEMA_ALIGNMENT = PASS
  # IDs max 128 + charset; max 256 endpoints; port≠0; weight/priority u32 range;
  # target+endpoints rejected; unknown fields follow existing RawConfig policy.
```

## Files touched (product)

- `config/ir/src/endpoint_set.rs` (new)
- `config/ir/src/lib.rs`, `error.rs`, `canonical.rs`
- `config/merge/src/lib.rs`
- `config/schema/src/lib.rs`
- `crates/exyonq-runtime-plan/src/runtime_plan.rs`, `lib.rs`
- `module-api/src/proxy_dispatch.rs`
- `crates/exyonq-mod-proxy/src/runtime.rs`, `upstream.rs`, `wire_conn.rs`
- Minimal call-site adaptations (compat/nginx, core tests, integration helpers)
- Docs: this file, ADR 035, k8s-p1 native contracts, gap analysis

## Non-goals (still closed)

Controller, Helm, CRDs, Go, Kubernetes crates/types, WRR hot-path picker, retries, active health, drain runtime, ADR-036 apply protocol productization, Cargo.toml/lock changes, opening K8S-P2B.
