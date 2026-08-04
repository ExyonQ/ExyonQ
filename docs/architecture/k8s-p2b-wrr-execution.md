# K8S-P2B — Productive Weighted Multi-Endpoint Execution

```text
DOCUMENT = docs/architecture/k8s-p2b-wrr-execution.md
PHASE = K8S-P2B
R4_REPLAY = PASS
OWNER_IMPLEMENTATION_ACCEPTANCE = ACCEPT_ON_CANONICAL_AFTER_R4
K8S_P2B_OPENED = YES
K8S_P2B_STATUS = CLOSED_REPLAYED_ON_CANONICAL
K8S_P2B_CLOSED = YES
K8S_P2B_CODE_ACCEPTED_FOR_LOCAL_COMMIT = YES
K8S_P2B_PRODUCTIVE_RELEASE_ACCEPTED = YES_R4
K8S_P2B_WRR_IMPLEMENTED = YES
K8S_P2B_PRIORITY_BANDS_IMPLEMENTED = YES
K8S_P2B_MULTI_ENDPOINT_EXECUTION_IMPLEMENTED = YES
K8S_P2B_RETRIES_IMPLEMENTED = NO
K8S_P2B_HEALTH_CHECKS_IMPLEMENTED = NO
K8S_P2C_OPENED = NO
K8S_P2C_READY_TO_ADMIT = YES_PENDING_OWNER_ADMISSION
K8S_P3_OPENED = NO
K8S_P6_OPENED = NO
P14V043_OPENED = NO
STRATEGY_HEAD = b70148e8edf0b9cfa043ecdf7031a1070fbc4c8d
R4_BASELINE = 0fed18db79c386e5868718a99dbe25582c4b7adf
OPEN_002_FIX = 1d815feabce134712ccd10af5b1ac5533f5d13af
COMMIT_AUTHORIZED = YES
PUSH_AUTHORIZED = NO
EXYONQ_IMPLEMENTATION_LANGUAGE_POLICY = RUST_ONLY
GO_CODE = NO
```

## Decisions

```text
WRR_ALGORITHM_SELECTED = SMOOTH_WEIGHTED_ROUND_ROBIN
WRR_STATE_MODEL = PER_BAND_CURRENT_WEIGHTS_MUTEX
WRR_SELECTION_COMPLEXITY = O_N_WITHIN_ACTIVE_PRIORITY_BAND_MAX_256
WRR_UPDATE_COMPLEXITY = O(n_band)
WRR_CONCURRENCY_MODEL = MUTEX_PER_PRIORITY_BAND
WRR_OVERFLOW_MODEL = NORMALIZE_WEIGHTS_TO_I64_WITH_SCALE
SELECTOR_LOCK_GRANULARITY = PER_BAND
SELECTOR_LOCK_SCOPE = SELECTION_STATE_ONLY
LOCK_HELD_DURING_NETWORK_IO = NO
CROSS_BACKEND_LOCK_CONTENTION = NO_BY_DESIGN_AND_TEST
GLOBAL_HOT_PATH_LOCKS_ADDED = NO
UNSAFE_ADDED = NO
WRR_ACCUMULATOR_BOUNDED = YES
WRR_WEIGHT_NORMALIZATION_TESTS = PASS
WRR_GENERATION_REFERENCE_SAFETY = PASS

PRIORITY_ORDERING = LOWER_NUMERIC_PRIORITY_IS_PREFERRED
PRIORITY_FAILOVER_TRIGGER = PREFERRED_BAND_HAS_ZERO_ELIGIBLE_AT_SELECTOR_BUILD
PRIORITY_RECOVERY_BEHAVIOR = ON_GENERATION_REBUILD_WHEN_PREFERRED_ELIGIBLE_AGAIN
P2B_PRIORITY_FAILOVER = CONFIGURED_ELIGIBILITY_ONLY

WEIGHT_ZERO_SEMANTICS = NOT_SELECTABLE
EMPTY_ENDPOINT_SET_BEHAVIOR = 503_SERVICE_UNAVAILABLE
NO_ELIGIBLE_ENDPOINT_BEHAVIOR = 503_SERVICE_UNAVAILABLE
FIRST_ENDPOINT_FALLBACK = FORBIDDEN
SILENT_ENDPOINT_TRUNCATION = NO
HTTP_RETRY_IMPLEMENTATION = DEFERRED
HEALTH_CHECKS = NO

# OPEN-002 (R4C) — Single vs Multi gated on *eligible* count
ELIGIBLE = admin_enabled AND weight > 0
SINGLE_VS_MULTI_GATE = eligible_count
configured >= 2 AND eligible == 1 → SingleEndpointReady / ClusterBinding::Single
preferred_band_size == 1 AND eligible >= 2 → MultiEndpointReady (do not collapse)

POOL_KEY_MODEL = ENDPOINT_TRANSPORT_IDENTITY
  # scheme://host:port of selected endpoint URI
POOL_CROSS_ENDPOINT_REUSE = FORBIDDEN
POOL_SAME_ENDPOINT_REUSE = ALLOWED
POOL_FUTURE_TLS_IDENTITY_GAP = DOCUMENTED
  # SNI/cert identity not yet a separate pool dimension; scheme/host/port already separable

SELECTOR_GENERATION_ID = RuntimeSnapshot.generation / bind generation
SELECTOR_REBUILD_TRIGGER = bind_compiled_slots
SELECTOR_AND_TARGET_TABLE_ATOMIC_COHERENCE = PASS
CROSS_GENERATION_INDEX_MIXING = NO
REMOVED_ENDPOINT_NEW_SELECTION = FORBIDDEN

WRR_TEST_SAMPLE_SIZE = 10000
WRR_TEST_TOLERANCE = ~5% equal; ~0.15 abs on 1:3 ratio; ~0.5 on 1:10
WRR_TEST_DETERMINISTIC = YES

ALLOCATIONS_PER_SELECTION =
  EXPECTED_ZERO_EXTRA_BEYOND_TARGET_CLONE_BY_CODE_INSPECTION_NOT_INSTRUMENTED
```

## Eligibility (build-time)

Eligible iff `admin_enabled && weight > 0`.
`Disabled` and `DrainRequested` are not eligible. No active health.

## HTTP status mapping

```text
NO_ELIGIBLE_ENDPOINT (before connect) = 503 ServiceUnavailable
  # dispatch, WebSocket helper path, cache load passthrough
SELECTED_THEN_CONNECT_REFUSED = 502 BadGateway (existing)
SELECTED_THEN_TIMEOUT = 504 GatewayTimeout (existing)
```

## P2A-OPEN-001

```text
P2A_OPEN_001_STATUS = CLOSED
P2A_OPEN_001_EVIDENCE = [
  property_endpoint_ordering_permutations_normalize_equivalently,
  property_invalid_ids_rejected_corpus,
  property_duplicate_ids_rejected,
  property_target_endpoints_ambiguity_rejected,
  property_oversized_endpoint_set_rejected,
  property_address_forms_ipv4_ipv6_hostname,
  property_serialization_round_trip_stable,
  property_deterministic_fingerprint_and_id,
  property_extreme_weights_priorities_no_panic,
  property_no_panic_under_repeated_normalize,
]
```

## Finding P2B-OPEN-001

```text
P2B-OPEN-001
TITLE = Productive WRR selector requires Linux performance and contention protectors before K8S-P2B closure.
STATUS = CLOSED
SEVERITY = HIGH
CLOSED_BY = strategy dual-arch protectors run k8s-p2b-prot-20260731T154407Z (Netcup amd64 + Oracle arm64)
NOTE = HISTORICAL_ON_STRATEGY_HEAD_b70148e; not re-executed on post-OPEN-002 tip (OPEN-002 is arch-independent eligibility gating)
BLOCKS_LOCAL_COMMIT = NO
BLOCKS_P2B_CLOSE = NO
BLOCKS_P2C_OPEN = NO
BLOCKS_RELEASE_CLAIMS = NO
```

## Finding P2B-OPEN-002 (R4)

```text
P2B-OPEN-002 / OPEN-002
TITLE = Multi binding reachable when effective selectable endpoints == 1
STATUS = CLOSED
SEVERITY = MEDIUM
ROOT_CAUSE = compile_proxy_compiled_slots gated Single on configured count, not eligible count
FIX = eligible==1 → SingleEndpointReady / ClusterBinding::Single; full endpoints[] retained
FIX_COMMIT = 1d815feabce134712ccd10af5b1ac5533f5d13af
FIX_REQUIRED = YES
SECURITY_IMPACT = NONE_MATERIAL
```

## Gates

```text
ARCHITECTURE_GATE = CONDITIONAL_ACCEPT_OPEN002_CLOSED (arch-compiler R4)
CORRECTNESS_GATE = PASS_FAIL_CLOSED
SECURITY_GATE = ACCEPT (security-compiler R4 OPEN-002)
PERFORMANCE_GATE = PASS_HISTORICAL_P2B_OPEN_001_STRATEGY
```

## Product claims

P2B productive WRR is accepted on the canonical R4 replay tip after OPEN-002.
Gateway API / controller / Helm / CRD remain out of scope until K8S-P2C owner admission.
No Go.
