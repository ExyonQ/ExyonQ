# K8S-P1 — Native Contracts Design

```text
DOCUMENT = docs/architecture/k8s-p1-native-contracts.md
PHASE = K8S-P1
OWNER_ACCEPTANCE = ACCEPT
K8S_P1_STATUS = CLOSED_ACCEPTED
K8S_P1_CLOSED = YES
K8S_P1_OPENED = YES
K8S_P1_KIND = CONTRACT_AND_CONFIG_IR_DESIGN_ONLY
K8S_P1_PRODUCT_IMPLEMENTATION_AUTHORIZED = NO
K8S_P1_CONTROLLER_IMPLEMENTATION_AUTHORIZED = NO
K8S_P1_GO_MODULE_AUTHORIZED = NO
K8S_P1_HELM_AUTHORIZED = NO
K8S_P1_CRDS_AUTHORIZED = NO
K8S_P1_CARGO_DEPENDENCY_CHANGES_AUTHORIZED = NO
K8S_P1_CARGO_LOCK_CHANGES_AUTHORIZED = NO
K8S_P1_COMMIT_AUTHORIZED = NO
K8S_P1_PUSH_AUTHORIZED = NO
K8S_P2_OPENED = YES
K8S_P2A_STATUS = CLOSED_ACCEPTED_FOUNDATION_RUNTIME_EXECUTION_DEFERRED
K8S_P2A_CLOSED = YES
K8S_P2B_OPENED = NO
K8S_P2B_READY_TO_ADMIT = YES_FOR_SEPARATE_WRR_EXECUTION_SCOPE
K8S_P3_OPENED = NO
K8S_P6_OPENED = NO
P14V043_OPENED = NO

# P2A closed separately: EndpointSet IR + RuntimePlan foundation accepted;
# multi-endpoint execution deferred to K8S-P2B (not opened by this close).
# Implementation note: docs/architecture/k8s-p2a-endpoint-set-implementation.md

EXYONQ_IMPLEMENTATION_LANGUAGE_POLICY = RUST_ONLY
EXYONQ_PRIMARY_IMPLEMENTATION_LANGUAGE = RUST
EXYONQ_GO_IMPLEMENTATION_AUTHORIZED = NO
EXYONQ_GO_IMPLEMENTATION_ALLOWED = NO
EXYONQ_GO_CODE_ALLOWED = NO
EXYONQ_GO_TOOLCHAIN_ALLOWED = NO
EXYONQ_GO_MODULES_ALLOWED = NO
EXYONQ_GO_DEPENDENCIES_ALLOWED = NO
EXYONQ_GO_CONTROLLER = REJECTED_BY_OWNER
EXYONQ_HYBRID_GO_RUST_ARCHITECTURE = REJECTED_BY_OWNER
EXYONQ_K8S_CONTROLLER_LANGUAGE_RECOMMENDATION = RUST_CONTROLLER_OVER_RUST_NATIVE_CONTRACT
EXYONQ_K8S_CONTROLLER_LANGUAGE_DECISION_STATUS = OWNER_DECIDED_RUST_ONLY
EXYONQ_K8S_CONTROLLER_LANGUAGE_REVALIDATION_REQUIRED = NO
AUDIT_BASE_HEAD = 1ed520e58bdaa5e892098e25ba523e913699d804
WORKTREE = exyonq-lab-wt-k8s-wedge
BRANCH = strategy/exyonq-k8s-wedge
```

## Purpose

Design **native, Kubernetes-agnostic** contracts so ExyonQ can manage multi-endpoint backends, load balancing, multi-certificate TLS, and atomic updates from many control surfaces (native config, CLI, Control API, Nexus, hosting panels, multi-node, future K8s controller, NGINX/ingress-nginx migrators).

```text
KUBERNETES = CONSUMER_OF_CONTRACTS
KUBERNETES ≠ OWNER_OF_DATA_PLANE_MODEL
EXYONQ_K8S_TYPES_IN_NATIVE_CONTRACTS = FORBIDDEN
```

Companion docs:

- `docs/architecture/k8s-p1-config-ir-gap-analysis.md`
- `docs/adr/035-native-endpoint-set-contract.md`
- `docs/adr/036-control-plane-data-plane-apply-contract.md`

---

## Normative principles

1. No exclusive Kubernetes types/names/semantics in native contracts.
2. Forbidden in `exyonq-config-ir`, `exyonq-runtime-plan`, `exyonq-core`, shared modules: GatewayClass, Gateway, HTTPRoute, GRPCRoute, BackendRef, EndpointSlice, Service, Secret, Namespace, ReferenceGrant, ObjectMeta, resourceVersion, Kubernetes UID, and any foreign Kubernetes client types (including names historically associated with client-go or kube-rs APIs). Product implementation language remains Rust-only.
3. Future K8s controller translates cluster resources → these contracts.
4. Contracts must be useful without Kubernetes.
5. No duplication of routing/LB/TLS/health/retries/observability in adapters.
6. Config contracts prefer secure references over persistent private material.
7. Invalid updates retain last valid generation.
8. Compatible schema evolution required.
9. Hot path must not depend on control-plane transport format.
10. Design semantic model before choosing REST/gRPC/files.

---

## Current reality inventory (summary)

Package `exyonq-config-ir` lives at `config/ir/` (not `crates/exyonq-config-ir/`).

| Area | Reality at `1ed520e` |
|------|----------------------|
| Upstream | `UpstreamConfig { name, target, timeout_ms }` — single HTTP target |
| Server/listener | `ServerConfig { listen, server_name, routes, tls, http3_listen }` |
| TLS | `TlsConfig { cert, key, acme? }` — paths; rustls `with_single_cert` |
| Route match | `RouteMatch { path, host? }` — longest path; optional host/wildcard suffix |
| Discovery | `DiscoveryCluster.endpoints` collapsed via `endpoints.first()` |
| Reload | generation counter + IR/snapshot fingerprint; TLS may mutate before state publish |
| Ops | unix socket: reload/status/drain/shutdown (`exyonq-ops-runtime`) |
| Control API product | Not a separate REST surface; ops socket + `exyonqctl` |

Full findings matrix: see gap-analysis document.

---

## CONTRACT 1 — Backend identity

### Alternatives evaluated

| Option | Summary | Verdict |
|--------|---------|---------|
| A. User-provided ID | Operator/panel supplies stable name | Base requirement |
| B. Content-derived ID | Hash of endpoints/policy | Useful fingerprint, unstable as rename identity |
| C. Internally generated ID | UUID/ulid assigned by DP | Good for anonymous objects; poor for human ops |
| D. Hybrid | Required user `backend_id` + optional content fingerprint + optional `source_id` | **SELECTED** |

### Decision

```text
EXYONQ_NATIVE_BACKEND_IDENTITY_MODEL = HYBRID_USER_ID_PLUS_FINGERPRINT_PLUS_SOURCE

BACKEND_ID = required opaque string; stable across generations; ASCII subset; max length PROVISIONAL 128
ENDPOINT_ID = required within set; stable across address changes when producer supplies it; else derived from normalized address+port+protocol
DISPLAY_NAME = optional UTF-8 label for UI; never used as join key
SOURCE_ID = optional producer-scoped opaque id (panel object id, migrator id, adapter id) — NOT a Kubernetes UID
STABLE_IDENTITY_RULE = BACKEND_ID is authority; rename requires explicit replace or alias map (default: rename = new id)
COLLISION_RULE = duplicate BACKEND_ID in one snapshot → REJECT
RENAME_SEMANTICS = treat as delete+create unless producer sends rename op in a future overlay API
METRICS_ID = low-cardinality token derived from BACKEND_ID (hash truncate or registry index) — not raw path/IP
LOGGING_ID = BACKEND_ID (or METRICS_ID) + optional DISPLAY_NAME; never Secret material
```

Forbidden as **mandatory** native identity: `namespace/service`, pod UID, EndpointSlice UID. Adapters may store those only inside `SOURCE_ID` / metadata opaque bags.

### Fingerprint stability (owner rectification)

```text
CONTENT_FINGERPRINT_INPUTS = desired configuration only
CONTENT_FINGERPRINT_EXCLUDES =
  runtime health | ready flags | ejected flags | connection counts |
  observed EndpointSlice resourceVersion | pod UIDs | volatile timestamps
SOURCE_ID = opaque producer correlation only; not part of LB selection
```

If a producer embeds volatile runtime data into metadata that affects the canonical hash, that is a producer bug: metadata used in fingerprint must be declared non-volatile or excluded from hash.

---

## CONTRACT 2 — Endpoint set

```text
EXYONQ_NATIVE_ENDPOINT_SET_MODEL = EXPLICIT_ENDPOINT_SET_WITH_STABLE_IDS
```

Conceptual model (illustrative, not implementation authorization):

```text
// CONCEPTUAL ONLY — IMPLEMENTATION_AUTHORIZED = NO
// Not Rust or Go source; illustrative field layout only.
EndpointSet {
    backend_id: BackendId,
    generation: u64,              // set-local or snapshot-scoped
    policy: LoadBalancingPolicy,
    endpoints: [Endpoint],        // ordered deterministically after normalize
}

Endpoint {
    endpoint_id: EndpointId,
    address: EndpointAddress,     // IpAddr or Hostname
    port: u16,
    protocol: UpstreamProtocol,   // HttpCleartext initially
    weight: u32,                  // 0 = never selected
    priority: u32,                // lower = preferred group; failover by priority band
    admin_state: AdminEndpointState, // Enabled | Disabled | DrainRequested
    // NO dynamic health fields in config IR
    metadata: map[string,string], // non-operative; size-limited
    source_id: Option<SourceId>,
}
```

### Field semantics

| Key | Definition |
|-----|------------|
| ENDPOINT_SET_IDENTITY | `backend_id` |
| ENDPOINT_IDENTITY | `endpoint_id` within set |
| ADDRESS_MODEL | IPv4, IPv6, or hostname; normalized lowercase hostname; reject empty |
| PORT_MODEL | `u16` 1..=65535 |
| PROTOCOL_MODEL | MVP: `http` cleartext; future: `https`, `h2c`, `h2` — versioned capabilities |
| WEIGHT_MODEL | `u32`; `0` = excluded from selection; default `1` |
| PRIORITY_MODEL | `u32`; selection within highest-priority non-empty ready band first |
| READINESS_MODEL | **admin** readiness in IR (`Enabled`/`Disabled`); **runtime** readiness separate |
| DRAIN_MODEL | admin `DrainRequested` in IR or control op; runtime drain progress separate |
| METADATA_MODEL | optional string map; not interpreted by hot path |
| MAX_ENDPOINTS | PROVISIONAL soft default 64; hard max 1024 per backend (candidate) |
| EMPTY_SET_SEMANTICS | valid config object; runtime returns configured empty-backend error (no implicit localhost) |
| DUPLICATE_SEMANTICS | duplicate `endpoint_id` → REJECT; duplicate address:port:protocol with distinct ids → ALLOW with warn |
| ORDERING_SEMANTICS | after validate, order by `(priority asc, endpoint_id asc)` for determinism |

Updates: MVP prefers **full set replace** per backend inside a snapshot. Incremental endpoint patch is CONTRACT_ONLY for a later producer API, not required for first IR.

---

## CONTRACT 3 — Load-balancing policy

```text
EXYONQ_NATIVE_LB_MVP = WEIGHTED_ROUND_ROBIN_WITH_PRIORITY_BANDS
LOAD_BALANCING_ALGORITHM = WEIGHTED_ROUND_ROBIN   # equal weights ⇒ round-robin
FAILOVER_PRIORITY = SEPARATE_PRIORITY_BANDS       # not fused into the WRR counter
```

`priority` selects which **band** is eligible. Within the highest non-empty eligible band, `LOAD_BALANCING_ALGORITHM` runs. Do not implement a single rigid primitive that conflates failover with weighted selection.

| Algorithm / concern | Classification |
|-----------|----------------|
| round-robin (equal weight) | REQUIRED_FOR_FIRST_IMPLEMENTATION (WRR special case) |
| weighted round-robin | REQUIRED_FOR_FIRST_IMPLEMENTATION |
| failover priority bands | REQUIRED_FOR_FIRST_IMPLEMENTATION (orthogonal to WRR) |
| least-connections | CONTRACT_ONLY (schema may reserve enum) |
| consistent hashing | DEFERRED |
| random / power-of-two | DEFERRED |
| locality | DEFERRED |

### Selection interactions

| Condition | Behavior |
|-----------|----------|
| weight 0 | never selected |
| admin disabled | never selected |
| draining | no **new** requests; existing may continue per drain contract |
| passive/active unhealthy / ejected | runtime excludes; not IR fields |
| empty selectable set | fail request with structured empty-backend error |
| generation change | rebuild precomputed picker off request path; swap atomically with snapshot |
| future affinity | must key on BACKEND_ID/ENDPOINT_ID, not EndpointSlice |

No algorithm may depend on Kubernetes EndpointSlice types.

---

## CONTRACT 4 — Health state

```text
EXYONQ_NATIVE_HEALTH_STATE_MODEL = DESIRED_ADMIN_IN_IR_PLUS_OBSERVED_RUNTIME_STATE
```

| State | Belongs in | Authority |
|-------|------------|-----------|
| administratively enabled/disabled | IR / desired | config producer |
| drain requested | IR or control op | producer / operator |
| ready (runtime) | runtime observed | data plane (+ adapter hints) |
| draining (in progress) | runtime observed | data plane |
| passive unhealthy | runtime | data plane |
| active unhealthy | runtime | data plane |
| ejected | runtime | data plane |
| unknown | runtime | data plane |

```text
DESIRED_CONFIGURATION ≠ OBSERVED_RUNTIME_STATE
K8s Ready condition = adapter input hint only; NOT the sole native health model
Persistence of dynamic health across process restart = NO by default (re-observe)
Adapter may publish observed hints via a side channel; must not write them into canonical Config IR
```

---

## CONTRACT 5 — Endpoint draining

```text
EXYONQ_NATIVE_DRAIN_MODEL = BEST_EFFORT_ADMIN_DRAIN_WITH_DEADLINE
```

Honest limit: current runtime has process-level drain (`lifecycle.rs`) but **not** per-endpoint drain. Contract must not promise stronger semantics than P2 can deliver.

| Key | Definition |
|-----|------------|
| DRAIN_REQUEST | mark endpoint/admin set draining via snapshot or control op |
| DRAIN_STATE | Requested → InProgress → Complete \| TimedOut \| Cancelled |
| DRAIN_DEADLINE | mandatory timeout; PROVISIONAL default 30s; hard max candidate 300s |
| NEW_REQUEST_BEHAVIOR | do not select draining endpoint |
| EXISTING_CONNECTION_BEHAVIOR | allow in-flight to finish until deadline; then reset/close |
| REMOVAL_ACK | runtime signals removable when no active streams/conns attributed (best-effort accounting) |
| TIMEOUT_BEHAVIOR | force-remove from selectable set; may abort remaining; emit metric/log |

Protocol notes (design intent for P2+, not promises of current code):

- HTTP/1.1 keep-alive: stop assigning; idle conns closable
- HTTP/2: stop new streams; existing streams until deadline
- WebSocket/SSE: treat as long-lived; drain may wait until deadline then close
- HTTP/3: same best-effort stream accounting if available
- Retries: must not retry onto a just-removed endpoint without re-picking from new set
- Generation update may combine drain + removal in one snapshot

---

## CONTRACT 6 — Route → backend references

```text
EXYONQ_NATIVE_ROUTE_BACKEND_REFERENCE_MODEL = ROUTE_TO_BACKEND_ID_TO_ENDPOINT_SET
```

Preferred:

```text
Route.action.proxy.backend_ref = BackendId
BackendId → EndpointSet
```

Optional weighted multi-backend at route level (canary across backends) as list of `(BackendId, weight)` — CONTRACT_ONLY for first IR if endpoint weights already cover canary; **recommend** endpoint weights inside one backend for MVP, route-level backend split as soon as Gateway canary needs distinct Services.

| Case | Behavior |
|------|----------|
| backend missing | REJECT snapshot |
| backend empty | ACCEPT config; runtime empty-backend error |
| duplicate backend refs | ALLOW |
| cycles / groups | REJECT until group feature admitted |
| reload change | atomic snapshot swap; in-flight may finish on old set |
| native vs adapter conflict | resolved by producer ownership model (Contract 13) |

Do **not** embed raw endpoint lists on every route.

---

## CONTRACT 7 — Timeout model

```text
EXYONQ_NATIVE_TIMEOUT_MODEL = SPLIT_TIMEOUTS_WITH_EXPLICIT_UNITS_MS
```

Replace ambiguous single `timeout_ms` as **final** contract. Migration: keep `timeout_ms` as deprecated alias mapping to `total_request_deadline_ms` until schema major bump completes.

| Timeout | Unit | Default (PROVISIONAL) | Min | Max candidate | `0` | null/absent |
|---------|------|------------------------|-----|---------------|-----|-------------|
| connect_ms | ms | 500 (align Hyper client today) | 1 | 120_000 | REJECT | use default |
| request_response_ms | ms | 30_000 | 1 | 600_000 | REJECT | use default |
| idle_ms | ms | pool/idle policy | 1 | 600_000 | means no idle reuse where applicable | default |
| upstream_read_ms | ms | inherit request_response | 1 | 600_000 | REJECT | inherit |
| upstream_write_ms | ms | inherit request_response | 1 | 600_000 | REJECT | inherit |
| total_request_deadline_ms | ms | 30_000 | 1 | 600_000 | REJECT | default |
| websocket_sse_ms | ms | 300_000 candidate | 1 | 3_600_000 | REJECT | default |
| drain_ms | ms | 30_000 | 1 | 300_000 | REJECT | default |
| health_check_ms | ms | DEFERRED | — | — | — | — |
| control_apply_ms | ms | 5_000 candidate | 1 | 60_000 | REJECT | default |

---

## CONTRACT 8 — Retry policy (contract only)

```text
EXYONQ_NATIVE_RETRY_MODEL = EXPLICIT_OPT_IN_IDEMPOTENT_ONLY
IMPLEMENTATION_AUTHORIZED = NO
```

| Field | Intent |
|-------|--------|
| max_attempts | inclusive original try; PROVISIONAL default 1 (no retry) |
| retry_on | connect_failure, reset_before_headers, selected 5xx (configurable), timeout — never opaque |
| methods | default safe/idempotent only (GET/HEAD/OPTIONS); others require explicit `allow_non_idempotent=true` **and** `body_replayable=true` |
| body_replayability | required flag for any method with body |
| budget | optional max retries/sec per backend |
| backoff | none \| fixed \| exponential with jitter |
| per_try_timeout_ms | optional |
| health interaction | failures feed passive health; ejected endpoints skipped |
| observability | count retries, reasons; no payload logging |

Silent retries of non-idempotent requests: **FORBIDDEN**.

---

## CONTRACT 9 — TLS certificate set

```text
EXYONQ_NATIVE_TLS_CERTIFICATE_SET_MODEL = CERTIFICATE_SET_WITH_REFERENCE_PROVIDERS
```

Separate:

| Concept | Role |
|---------|------|
| TLS_CERTIFICATE_REFERENCE | id + SNI names + default flag + provider ref |
| TLS_CERTIFICATE_MATERIAL_PROVIDER | file \| acme \| memory_control \| external_secret_provider \| future_k8s_adapter |
| TLS_RUNTIME_MATERIAL | loaded bytes in memory only; never in IR canonical hash as raw key |

Rules:

- Multiple certs per listener/server group; SNI select; one default.
- Exact names and wildcards with documented precedence (exact > longer wildcard > default).
- Duplicate SNI → REJECT.
- Invalid cert → REJECT snapshot (or DEGRADED listener if partial programming admitted later — MVP: REJECT).
- Rotation: new material via provider; atomic acceptor swap (existing `SharedTlsAcceptor::load` pattern).
- Native IR must **not** require `Secret` / `secretName`.
- EXYONQ-SEC-PRIVATE-MATERIAL-ZERO: no private keys in logs, reports, debug, versioned snapshots.

---

## CONTRACT 10 — Generation and snapshot

```text
EXYONQ_NATIVE_GENERATION_MODEL = HYBRID_PRODUCER_EPOCH_PLUS_DP_MONOTONIC_APPLY_GENERATION
```

| Field | Role |
|-------|------|
| schema_version | contract major.minor |
| content_fingerprint | hash of canonical IR/snapshot bytes |
| source_revision | optional producer revision string |
| producer_id | which producer authored |
| producer_epoch | producer-assigned monotonic token (optional) |
| apply_generation | data-plane assigned on successful apply |
| created_at | optional timestamp metadata |

```text
last_good = last successfully APPLIED snapshot
candidate = STAGED but not ACK
active = currently serving
rejected = failed validate/apply; last_good retained
degraded = partial program (future; MVP prefers reject)
```

Multiple producers: see Contract 13. Replay of identical fingerprint is idempotent no-op.

### Restart and stale producer epochs

```text
DP_APPLY_GENERATION = monotonic in-process; may reset on process restart
LAST_GOOD_FINGERPRINT = durable across restart (on staging media)
PRODUCER_EPOCH = producer-assigned; opaque monotonic token per producer_id
STALE_EPOCH_DETECTION =
  if producer_id matches authoritative producer AND
  producer_epoch < last accepted epoch for that producer → REJECT as PRODUCER_STALE
AFTER_DP_RESTART =
  reload last_good fingerprint into active;
  apply_generation restarts from 1 (or persisted counter if later authorized);
  producers must resync via STATUS before APPLY with EXPECTED_CURRENT_GENERATION
```

---

## CONTRACT 11 — Apply protocol

```text
EXYONQ_NATIVE_APPLY_PROTOCOL = VALIDATE_STAGE_APPLY_ACK_WITH_LAST_GOOD
EXYONQ_NATIVE_INITIAL_APPLY_TRANSPORT = HYBRID_ATOMIC_FILE_PLUS_UNIX_CONTROL_ACK
```

Semantic ops (transport-independent): VALIDATE, STAGE, APPLY, ACK, REJECT, ROLLBACK, STATUS.

Conceptual request fields: `APPLY_ID`, `SNAPSHOT_ID`, `SCHEMA_VERSION`, `EXPECTED_CURRENT_GENERATION`, payload/ref, producer_id, timeout.

Conceptual response: `RESULTING_GENERATION`, `ACK_STATUS`, `ERROR_CODE`, `ERROR_PATH`, `RETRYABILITY`, `ACTIVE_FINGERPRINT`, `LAST_GOOD_FINGERPRINT`.

States: RECEIVED, VALIDATED, STAGED, APPLIED, REJECTED, ROLLED_BACK, STALE, CONFLICT.

### Transport comparison

| Option | BENEFITS | RISKS | ATOMICITY | ACK_SUPPORT | SECURITY | CROSS_PLATFORM | PANELS | K8S | MULTINODE | RECOMMENDATION |
|--------|----------|-------|-----------|-------------|----------|----------------|--------|-----|-----------|----------------|
| A atomic file + watcher | Simple; fits today reload | weak ACK; TOCTOU | good with rename | weak | FS perms | good | medium | good via emptyDir | weak | USE as material carrier |
| B Unix socket | Exists (`ops-runtime`) | local only | via commands | strong | SO_PEERCRED later | Unix | good local | via sidecar | weak | **ACK channel MVP** |
| C REST local | panel-friendly | auth surface | needs design | strong | TLS/auth | good | **best** | possible | medium | EVOLVE Control API |
| D gRPC | streaming status | complexity | good | strong | mTLS | good | medium | good | **best later** | DEFER |
| E hybrid file+socket | reuse now | two mechanisms | strong | strong | combine | good | good | good | medium | **SELECT INITIAL** |

### Unix ACK authority (owner rectification)

```text
APPLY_RESULT_AUTHORITY = UNIX_CONTROL_ACK_RESPONSE
FILE_CARRIER = candidate snapshot bytes only; not success proof
ACK_CORRELATION = APPLY_ID (and SNAPSHOT_ID) must round-trip in ACK/REJECT
WITHOUT_CORRELATED_ACK = treat apply as incomplete; do not advance last_good
WATCHER_ONLY_SIGNAL = INSUFFICIENT
```

The unix control response is the authority for apply outcome. A filesystem watcher event without correlated `APPLY_ID` ACK is not a successful apply.

---

## CONTRACT 12 — Version negotiation

```text
EXYONQ_NATIVE_CONTRACT_VERSIONING = MAJOR_MINOR_PLUS_CAPABILITY_FLAGS
```

| Field | Rule |
|-------|------|
| CONTRACT_MAJOR | incompatible break |
| CONTRACT_MINOR | compatible add |
| CAPABILITIES | advertised by data plane |
| REQUIRED_FEATURES | producer requirements |
| OPTIONAL_FEATURES | best-effort |

Unknown required capability → REJECT (never silent).
Unknown optional field → ignore only if minor-compatible and explicitly allowed by schema policy.
Deprecated fields: warn; map; remove on next major.

---

## CONTRACT 13 — Multiple config producers

```text
EXYONQ_NATIVE_MULTIPLE_PRODUCER_MODEL =
  INITIAL_SINGLE_AUTHORITATIVE_PRODUCER_PER_INSTANCE
  + FUTURE_PARTITION_OWNERSHIP
```

| Model | Verdict |
|-------|---------|
| A single authoritative producer | **INITIAL SELECT** |
| B namespace/partition ownership | **FUTURE** |
| C overlay layers | limited (discovery overlay today); danger if unbounded |
| D transaction merge | REJECT for MVP complexity |
| E reject concurrent producers | enforce with producer lease / generation expect |

Rules:

- One authoritative producer id per ExyonQ instance initially (file\|cli\|api\|panel\|k8s-controller).
- K8s controller must not overwrite unrelated standalone config without explicit ownership claim.
- Conflicts: `EXPECTED_CURRENT_GENERATION` mismatch → CONFLICT; retain last_good.
- Audit: every snapshot carries `producer_id` + fingerprint.
- Rollback: ROLLBACK to last_good or explicit prior fingerprint if retained.

---

## CONTRACT 14 — Observability identity

```text
EXYONQ_NATIVE_OBSERVABILITY_IDENTITY_MODEL = STABLE_ID_PLUS_BOUNDED_CARDINALITY_TOKEN
UNBOUNDED_ROUTE_LABELS = FORBIDDEN
UNBOUNDED_BACKEND_LABELS = FORBIDDEN
RAW_POD_IP_AS_UNBOUNDED_METRIC_LABEL = FORBIDDEN
SECRET_DATA_IN_METRICS = FORBIDDEN
```

| Entity | Stable ID | Metric token | Display |
|--------|-----------|--------------|---------|
| route | route name/id | capped registry / hash16 | name |
| backend | BACKEND_ID | capped | DISPLAY_NAME |
| endpoint set | BACKEND_ID | same | — |
| endpoint | ENDPOINT_ID | optional debug only | address redacted in metrics |
| listener | listener id | capped | listen addr |
| certificate set | cert_set_id | capped | SNI list in logs carefully |
| snapshot | fingerprint + generation | generation gauge | — |
| producer | producer_id | low cardinality enum/string | — |

---

## CONTRACT 15 — Limits and DoS safety

```text
EXYONQ_NATIVE_LIMITS_MODEL = PROVISIONAL_DEFAULTS_WITH_HARD_MAX_REJECT
```

All numeric values **PROVISIONAL** candidates pending evidence:

| Limit | DEFAULT | HARD_MAX | CONFIGURABLE | REJECTION_BEHAVIOR | OBSERVABILITY | SECURITY_REASON |
|-------|---------|----------|--------------|--------------------|---------------|-----------------|
| listeners | 16 | 256 | YES | REJECT snapshot | count | FD/exhaustion |
| routes | 256 | 10_000 | YES | REJECT | count | CPU match |
| backends | 128 | 4_096 | YES | REJECT | count | memory |
| endpoints/backend | 64 | 1_024 | YES | REJECT | count | churn/CPU |
| total endpoints | 512 | 16_384 | YES | REJECT | count | memory |
| certificates | 32 | 1_024 | YES | REJECT | count | memory/TLS |
| hostnames/cert | 16 | 256 | YES | REJECT | count | SNI map |
| snapshot bytes | 1 MiB | 32 MiB | YES | REJECT | size | DoS |
| structure depth | 8 | 32 | NO | REJECT | — | parser safety |
| producers | 1 | 8 | YES future | CONFLICT | producer_id | ownership |
| apply frequency | 10/s soft | 100/s | YES | STALE/backoff | rate | CPU |
| endpoint churn | debounce 200–1000ms | — | YES | coalesce | churn metric | reload storms |
| errors/snapshot | — | 1_000 listed | NO | truncate detail | — | log DoS |

---

## CONTRACT 16 — Failure model

```text
EXYONQ_NATIVE_FAILURE_MODEL = STRUCTURED_ERROR_CODES_WITH_LAST_GOOD_RETENTION
```

| Failure | ERROR_CODE | USER_VISIBLE_MESSAGE (pattern) | SECRET_REDACTION | RETRYABLE | LAST_GOOD_BEHAVIOR | STATUS_IMPACT | RECOVERY_ACTION |
|---------|------------|--------------------------------|------------------|-----------|--------------------|---------------|-----------------|
| invalid snapshot | `SNAPSHOT_INVALID` | validation failed at path | redact key material | NO | retain | REJECTED | fix config |
| broken reference | `REF_MISSING` | backend/cert id missing | OK | NO | retain | REJECTED | fix refs |
| invalid certificate | `TLS_CERT_INVALID` | cert failed checks | no key bytes | NO | retain | REJECTED | replace cert |
| private key inaccessible | `TLS_KEY_UNAVAILABLE` | key not readable | no path secrets if sensitive | YES/NO | retain | REJECTED | fix provider perms |
| empty backend | `BACKEND_EMPTY` | no endpoints configured | OK | NO at apply; YES at request as 503 class | apply may ACCEPT | DEGRADED/OK | add endpoints |
| invalid endpoint | `ENDPOINT_INVALID` | bad address/port | OK | NO | retain | REJECTED | fix endpoint |
| schema incompatible | `SCHEMA_INCOMPATIBLE` | major mismatch | OK | NO | retain | REJECTED | upgrade DP or producer |
| apply timeout | `APPLY_TIMEOUT` | apply exceeded budget | OK | YES | retain | REJECTED/STALE | retry/backoff |
| partial reload | `RELOAD_PARTIAL` | subsystem apply incomplete | OK | YES | prefer rollback to last_good | DEGRADED | fix atomicity |
| runtime not responding | `RUNTIME_UNAVAILABLE` | control channel failed | OK | YES | unchanged | STALE | restart DP |
| producer stale | `PRODUCER_STALE` | expected generation mismatch | OK | YES | retain | CONFLICT | resync |
| generation conflict | `GENERATION_CONFLICT` | concurrent producers | OK | YES | retain | CONFLICT | elect owner |
| last_good corrupt | `LAST_GOOD_CORRUPT` | cannot load last_good | OK | NO | safe mode / empty reject | FATAL class | restore backup |

Opaque-only errors: **FORBIDDEN** as sole contract.

---

## Gates (design)

### Architecture
- K8s-agnostic contracts: PASS (by design)
- Core without K8s types: PASS (constraint)
- Shared capabilities once: PASS
- Desired vs runtime health: PASS
- Transport separated from semantics: PASS
- No adapter duplication: PASS
- Panels/Nexus/multinode compatible: PASS (producer model)

### Security
- PRIVATE_MATERIAL_ZERO: PASS
- No private keys in these docs/examples: PASS
- Apply authz in design: CONDITIONAL (socket/FS perms MVP; stronger later)
- DoS limits: PASS (provisional)
- Error redaction: PASS
- Rollback/last_good: PASS
- Producer ownership: PASS
- No snippets in native contracts: PASS

### Correctness
- Deterministic refs/order: PASS
- Generations/conflicts: PASS
- last_good: PASS
- Idempotence: PASS
- Semantic atomicity: CONDITIONAL until P2 hardens TLS/state ordering
- Structured errors: PASS (design)
- Legacy TOML compatibility path: PASS (deprecations)
- No ambiguous health-in-IR: PASS

### Performance (design readiness only)
- No unbounded linear search mandated on hot path: PASS (precompiled picker)
- No per-request recompile: PASS
- Endpoint selection precompilable: PASS
- Immutable snapshots: PASS
- Updates off request path: PASS
- Bounded cardinality: PASS

```text
PERFORMANCE_COMPILER = DESIGN_READINESS_ONLY
AGENT_EXECUTION = MANUAL_CHECKLIST_ONLY
```

---


---

## Rust-only product architecture (owner normative)

```text
EXYONQ_IMPLEMENTATION_LANGUAGE_POLICY = RUST_ONLY
EXYONQ_PRIMARY_IMPLEMENTATION_LANGUAGE = RUST
EXYONQ_GO_IMPLEMENTATION_AUTHORIZED = NO
EXYONQ_GO_IMPLEMENTATION_ALLOWED = NO
EXYONQ_GO_CODE_ALLOWED = NO
EXYONQ_GO_TOOLCHAIN_ALLOWED = NO
EXYONQ_GO_MODULES_ALLOWED = NO
EXYONQ_GO_DEPENDENCIES_ALLOWED = NO
EXYONQ_GO_CONTROLLER = REJECTED_BY_OWNER
EXYONQ_HYBRID_GO_RUST_ARCHITECTURE = REJECTED_BY_OWNER
ALTERNATIVE_GO_CONTROLLER = REJECTED_BY_OWNER_RUST_ONLY_POLICY

EXYONQ_RUST_ONLY_SCOPE =
  DATA_PLANE
  + CONTROL_API
  + K8S_CONTROLLER
  + GATEWAY_API_TRANSLATION
  + INGRESS_COMPAT
  + MIGRATION_TOOLING
  + POLICY_API
  + CLI
  + OPERATIONS_TOOLING
```

### Component reformulation

| Component | COMPONENT_TYPE | Notes |
|-----------|----------------|-------|
| `exyonq-core` | RUST_LIBRARY_CRATE (existing) | no K8s types |
| `exyonq-config-ir` | RUST_LIBRARY_CRATE at `config/ir` | native contracts |
| `exyonq-runtime-plan` | RUST_LIBRARY_CRATE (existing) | compile target |
| `exyonq-control-api` | RUST contracts/runtime when authorized | evolves ops-runtime |
| `exyonq-k8s-controller` | **RUST_BINARY_CRATE** (separate) | watches/reconcile/status/leader/apply |
| `exyonq-k8s-gateway` | Rust module/crate **inside** controller tree | translation |
| `exyonq-k8s-ingress-compat` | RUST_LIBRARY_CRATE shareable | controller + migrator |
| `exyonq-k8s-policy-api` | Rust types/schema outside core | later |
| `exyonq-ingress-migrate` | RUST_BINARY_CRATE | report-only migrator |

```text
RUST_CRATE_SPLIT_POLICY =
  SPLIT_ONLY_FOR_CLEAR_PUBLIC_CONTRACT_FAILURE_DOMAIN_OR_REUSE
```

Conceptual controller layout (design only — **do not create dirs/code now**):

```text
exyonq-k8s-controller/
  src/
    main.rs
    controller/
    gateway/
    ingress/
    reconcile/
    status/
    discovery/
    secrets/
    apply/
    leader/
    telemetry/
```

Future candidate deps (**NOT authorized now**): `kube`, `kube-runtime`, `k8s-openapi`, `serde`, `tokio`, `futures`, `tracing`, `rustls` if needed, Gateway API types in an isolated crate.

```text
DEPENDENCIES_ADDED = NO
CARGO_TOML_CHANGED = NO
CARGO_LOCK_CHANGED = NO
KUBERNETES_DEPS_IN_HOT_PATH = FORBIDDEN
KUBERNETES_DEPS_IN_GENERAL_CORE = FORBIDDEN
EXYONQ_K8S_TYPES_IN_NATIVE_CONTRACTS = FORBIDDEN
EXYONQ_K8S_CONTROLLER_PROCESS_BOUNDARY = SEPARATE_RUST_BINARY_OUTSIDE_CORE
EXYONQ_K8S_CONTROLLER_DEPENDENCY_BOUNDARY = KUBERNETES_CRATES_ONLY_IN_CONTROLLER_WORKSPACE_SCOPE
```

Controller remains: independent process; separate binary; outside `core/`; consumer of native contracts; no duplicated proxy/TLS/LB/retry/health logic; isolated via apply contract.

---

## Rust controller ecosystem evaluation (Go not reopened)

```text
RUST_CONTROLLER_FEASIBILITY = FEASIBLE_WITH_PRE_IMPLEMENTATION_SPIKES
EXYONQ_K8S_CONTROLLER_RUST_FEASIBILITY = FEASIBLE_WITH_PRE_IMPLEMENTATION_SPIKES
```

| Area | CURRENT_ECOSYSTEM_SUPPORT | EXPECTED_IMPLEMENTATION_APPROACH | KNOWN_RISK | MITIGATION | SPIKE_REQUIRED | BLOCKING_FOR_K8S_P3 |
|------|---------------------------|----------------------------------|------------|------------|----------------|---------------------|
| RUST_K8S_CLIENT_MODEL | Mature-enough via `kube`/`k8s-openapi` (to be validated) | typed client in controller crate only | API drift vs cluster | pin versions; spike | R1 | YES until R1 PASS |
| RUST_WATCH_MODEL | Supported by kube-runtime watchers | watch → event queue | missed events / bookmarks | resync + resourceVersion discipline | R1 | YES |
| RUST_REFLECTOR_MODEL | Store/reflector patterns exist | reflector+indexer | memory growth | namespace scoping; limits | R1/R5 | YES |
| RUST_RECONCILIATION_MODEL | Team-owned loop on tokio | queue+worker reconcile | thundering herd | debounce/coalesce | R1/R5 | YES |
| RUST_LEADER_ELECTION_MODEL | Lease-based examples exist; less standard than Go | coordination.k8s.io Lease | split brain | spike proven election | R4 | YES |
| RUST_STATUS_UPDATE_MODEL | Patch/status subresource via API | status patch with conditions | SSA/conflict | spike conditions patch | R3 | YES |
| RUST_GATEWAY_API_TYPE_MODEL | Community/generated types uneven | isolated gateway-api types crate or generate | CRD version skew | spike type compatibility | R2 | YES |
| RUST_CRD_TYPE_MODEL | k8s-openapi / custom | versioned structs | unknown fields | reject unknown required caps | R2 | PARTIAL |
| RUST_ERROR_MODEL | thiserror/anyhow in tree today | structured controller errors mapped to native codes | opaque errors | Contract 16 codes | — | NO (design ready) |
| RUST_BACKOFF_MODEL | common rust retry crates / custom | exp backoff+jitter | retry storms | caps + metrics | R1 | NO |
| RUST_DEBOUNCE_MODEL | custom | time-bucket coalesce per key | lag | tunable debounce | R5 | YES for churn |
| RUST_SHUTDOWN_MODEL | tokio CancellationToken patterns | drain watches; last_good coord | partial apply on signal | spike R7 | R7 | YES |
| RUST_TEST_MODEL | rust unit + kind local | mock + kind integration | flaky cluster tests | deterministic fakes first | R1–R3 | NO |
| RUST_SUPPLY_CHAIN_MODEL | cargo deny/audit existing culture | controller workspace isolation | kube dep CVEs | audit gate; pin | — | NO (process) |
| RUST_MAINTENANCE_MODEL | one language | single toolchain CI | kube-rs churn | spikes + pin policy | ongoing | NO |

Historical comparison to a Go controller is retained only as:

```text
ALTERNATIVE_GO_CONTROLLER = REJECTED_BY_OWNER_RUST_ONLY_POLICY
```

---

## Pre-K8S-P3 Rust spikes (defined, not executed)

```text
K8S_P3_RUST_SPIKES_REQUIRED = YES
SPIKES_EXECUTED_THIS_PHASE = NO
```

| Spike | OBJECTIVE | AUTHORIZED_SCOPE | DEPENDENCIES | SUCCESS_CRITERIA | FAILURE_CRITERIA | SECURITY_GATES | OUTPUT | PRODUCT_CODE_REUSABLE |
|-------|-----------|------------------|--------------|------------------|------------------|----------------|--------|------------------------|
| SPIKE_R1 | watch + reflector + minimal reconcile | disposable spike tree/docs evidence only when later authorized | kube stack candidates | continuous watch; resync OK | missed events unhandled | no cluster-admin | report | NO_UNLESS_LATER_AUTHORIZED |
| SPIKE_R2 | GatewayClass/Gateway/HTTPRoute types | spike only | gateway API CRDs | decode/roundtrip sample objects | incompatible fields blocking Core | no persist secrets | report | NO_UNLESS_LATER_AUTHORIZED |
| SPIKE_R3 | status subresource + conditions | spike only | apiserver | patch Accepted/Programmed | lost updates / races | RBAC minimal | report | NO_UNLESS_LATER_AUTHORIZED |
| SPIKE_R4 | Lease leader election | spike only | coordination.k8s.io | single leader under kill/restart | dual writers | least privilege | report | NO_UNLESS_LATER_AUTHORIZED |
| SPIKE_R5 | EndpointSlice churn debounce | spike only | EndpointSlice | coalesce storms; bounded CPU | reload thrash | — | report | NO_UNLESS_LATER_AUTHORIZED |
| SPIKE_R6 | Secret watch zero private-material logging | spike only | Secrets | rotate without key in logs/metrics | any key/PEM in output | PRIVATE_MATERIAL_ZERO | report | NO_UNLESS_LATER_AUTHORIZED |
| SPIKE_R7 | graceful shutdown + last_good apply coord | spike only | apply contract | correlated APPLY_ID ACK; last_good retained | uncorrelated success | — | report | NO_UNLESS_LATER_AUTHORIZED |
| SPIKE_R8 | memory/startup/load profile | spike only | — | baseline RSS/startup under load | unbounded growth | — | report | NO_UNLESS_LATER_AUTHORIZED |

---

## Verdict block

```text
OWNER_ACCEPTANCE = ACCEPT
K8S_P1_STATUS = CLOSED_ACCEPTED
K8S_P1_CLOSED = YES
K8S_P1_CONTRACT_READINESS = ACCEPTED_FOR_IMPLEMENTATION_ADMISSION
K8S_P1_CONFIG_IR_READINESS = ACCEPTED_FOR_IMPLEMENTATION_ADMISSION
K8S_P1_RUST_ONLY_RECONCILIATION = PASS
K8S_P1_BREAKING_SCHEMA_CHANGE_REQUIRED = YES

EXYONQ_IMPLEMENTATION_LANGUAGE_POLICY = RUST_ONLY
EXYONQ_PRIMARY_IMPLEMENTATION_LANGUAGE = RUST
EXYONQ_GO_IMPLEMENTATION_AUTHORIZED = NO
EXYONQ_GO_IMPLEMENTATION_ALLOWED = NO
EXYONQ_GO_CODE_ALLOWED = NO
EXYONQ_GO_TOOLCHAIN_ALLOWED = NO
EXYONQ_GO_MODULES_ALLOWED = NO
EXYONQ_GO_DEPENDENCIES_ALLOWED = NO
EXYONQ_GO_CONTROLLER = REJECTED_BY_OWNER
EXYONQ_HYBRID_GO_RUST_ARCHITECTURE = REJECTED_BY_OWNER
ALTERNATIVE_GO_CONTROLLER = REJECTED_BY_OWNER_RUST_ONLY_POLICY
ACTIVE_GO_RECOMMENDATIONS_REMAINING = 0

EXYONQ_NATIVE_BACKEND_IDENTITY_MODEL = HYBRID_USER_ID_PLUS_FINGERPRINT_PLUS_SOURCE
EXYONQ_NATIVE_ENDPOINT_SET_MODEL = EXPLICIT_ENDPOINT_SET_WITH_STABLE_IDS
EXYONQ_NATIVE_LB_SELECTION_POLICY = WEIGHTED_ROUND_ROBIN
EXYONQ_NATIVE_LB_FAILOVER_POLICY = PRIORITY_BANDS
EXYONQ_NATIVE_LB_MVP = WEIGHTED_ROUND_ROBIN_WITH_PRIORITY_BANDS
LOAD_BALANCING_ALGORITHM = WEIGHTED_ROUND_ROBIN
FAILOVER_PRIORITY = SEPARATE_PRIORITY_BANDS
EXYONQ_NATIVE_HEALTH_STATE_MODEL = DESIRED_ADMIN_IN_IR_PLUS_OBSERVED_RUNTIME_STATE
EXYONQ_NATIVE_DRAIN_MODEL = BEST_EFFORT_ADMIN_DRAIN_WITH_DEADLINE
EXYONQ_NATIVE_ROUTE_BACKEND_REFERENCE_MODEL = ROUTE_TO_BACKEND_ID_TO_ENDPOINT_SET
EXYONQ_NATIVE_TIMEOUT_MODEL = SPLIT_TIMEOUTS_WITH_EXPLICIT_UNITS_MS
EXYONQ_NATIVE_RETRY_MODEL = EXPLICIT_OPT_IN_IDEMPOTENT_ONLY
EXYONQ_NATIVE_TLS_CERTIFICATE_SET_MODEL = CERTIFICATE_SET_WITH_REFERENCE_PROVIDERS
EXYONQ_NATIVE_GENERATION_MODEL = HYBRID_PRODUCER_EPOCH_PLUS_DP_MONOTONIC_APPLY_GENERATION
EXYONQ_NATIVE_APPLY_PROTOCOL = VALIDATE_STAGE_APPLY_ACK_WITH_LAST_GOOD
EXYONQ_NATIVE_INITIAL_APPLY_TRANSPORT = HYBRID_ATOMIC_FILE_PLUS_UNIX_CONTROL_ACK
EXYONQ_NATIVE_APPLY_RESULT_AUTHORITY = UNIX_CONTROL_ACK_RESPONSE_CORRELATED_BY_APPLY_ID
APPLY_RESULT_AUTHORITY = UNIX_CONTROL_ACK_RESPONSE
EXYONQ_NATIVE_CONTRACT_VERSIONING = MAJOR_MINOR_PLUS_CAPABILITY_FLAGS
EXYONQ_NATIVE_MULTIPLE_PRODUCER_MODEL = INITIAL_SINGLE_AUTHORITATIVE_PRODUCER_PER_INSTANCE_PLUS_FUTURE_PARTITION_OWNERSHIP
EXYONQ_NATIVE_OBSERVABILITY_IDENTITY_MODEL = STABLE_ID_PLUS_BOUNDED_CARDINALITY_TOKEN
EXYONQ_NATIVE_LIMITS_MODEL = PROVISIONAL_DEFAULTS_WITH_HARD_MAX_REJECT
EXYONQ_NATIVE_FAILURE_MODEL = STRUCTURED_ERROR_CODES_WITH_LAST_GOOD_RETENTION

EXYONQ_K8S_TYPES_IN_NATIVE_CONTRACTS = FORBIDDEN
EXYONQ_K8S_CONTROLLER_LANGUAGE_RECOMMENDATION = RUST_CONTROLLER_OVER_RUST_NATIVE_CONTRACT
EXYONQ_K8S_CONTROLLER_LANGUAGE_DECISION_STATUS = OWNER_DECIDED_RUST_ONLY
EXYONQ_K8S_CONTROLLER_LANGUAGE_REVALIDATION_REQUIRED = NO
EXYONQ_K8S_CONTROLLER_RUST_FEASIBILITY = FEASIBLE_WITH_PRE_IMPLEMENTATION_SPIKES
EXYONQ_K8S_CONTROLLER_PROCESS_BOUNDARY = SEPARATE_RUST_BINARY_OUTSIDE_CORE
EXYONQ_K8S_CONTROLLER_DEPENDENCY_BOUNDARY = KUBERNETES_CRATES_ONLY_IN_CONTROLLER_WORKSPACE_SCOPE

K8S_P2_OPENED = NO
K8S_P3_OPENED = NO
K8S_P6_OPENED = NO
K8S_P2_READY_TO_ADMIT = YES_FOR_SEPARATE_SCOPE_PREPARATION
K8S_P3_CONTROLLER_LANGUAGE = RUST
K8S_P3_GO_TOOLCHAIN_REQUIRED = NO
K8S_P3_RUST_SPIKES_REQUIRED = YES
K8S_P3_READY_TO_ADMIT = NO_PENDING_K8S_P2_AND_RUST_CONTROLLER_SPIKES
K8S_P6_MIGRATOR_LANGUAGE = RUST
K8S_P6_READY_TO_ADMIT = YES_FOR_SEPARATE_DOCS_OR_IMPLEMENTATION_SCOPE

K8S_P1_RECOMMENDED_NEXT_ADMIT =
  K8S-P2 — Shared Data-Plane Contracts Implementation
  first tranche P2A — Config IR compatibility and endpoint-set foundation
  (BackendId/EndpointId, EndpointSet, selection vs failover policy,
   legacy UpstreamConfig.target sugar, validation, schema/versioning,
   RuntimePlan compile, tests; no controller/K8s/active health/retries/
   multi-SNI/premature perf)
  K8S-P3 remains blocked on Rust spikes R1–R8 + P2 progress
  K8S-P6 may be admitted separately for docs/implementation scope
  no Go toolchain/modules/code ever for product components
  THIS CLOSURE DOES NOT OPEN P2/P3/P6

IMPLEMENTATION_AUTHORIZED = NO
ADR_IMPLEMENTATION_AUTHORIZED = NO
PRODUCT_CODE_CHANGED = NO
```
