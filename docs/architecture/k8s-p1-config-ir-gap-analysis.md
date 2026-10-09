# K8S-P1 — Config IR / Runtime Gap Analysis

```text
DOCUMENT = docs/architecture/k8s-p1-config-ir-gap-analysis.md
PHASE = K8S-P1
OWNER_ACCEPTANCE = ACCEPT
K8S_P1_STATUS = CLOSED_ACCEPTED
K8S_P1_CLOSED = YES
K8S_P1_CONFIG_IR_READINESS = ACCEPTED_FOR_IMPLEMENTATION_ADMISSION
BASE_HEAD = 1ed520e58bdaa5e892098e25ba523e913699d804
IMPLEMENTATION_AUTHORIZED = NO
K8S_P2_OPENED = YES
K8S_P2A_STATUS = CLOSED_ACCEPTED_FOUNDATION_RUNTIME_EXECUTION_DEFERRED
K8S_P2A_CLOSED = YES
K8S_P2A_IR_IMPLEMENTED = YES
K8S_P2A_RUNTIME_PLAN_IMPLEMENTED = YES
K8S_P2A_MULTI_ENDPOINT_EXECUTION_IMPLEMENTED = NO
K8S_P2A_WRR_RUNTIME_IMPLEMENTATION = DEFER_TO_K8S_P2B
K8S_P2B_OPENED = NO
K8S_P3_OPENED = NO
K8S_P6_OPENED = NO
P14V043_OPENED = NO
P2A_OPEN_FINDING_ID = P2A-OPEN-001
P2A_OPEN_FINDING_BLOCKS_P2B_CLOSE = YES
EXYONQ_IMPLEMENTATION_LANGUAGE_POLICY = RUST_ONLY
EXYONQ_GO_CODE_ALLOWED = NO
EXYONQ_K8S_CONTROLLER_LANGUAGE_RECOMMENDATION = RUST_CONTROLLER_OVER_RUST_NATIVE_CONTRACT
ACTIVE_GO_RECOMMENDATIONS_REMAINING = 0
```

P2A close: EndpointSet IR gaps addressed for foundation (no silent truncation; full set in plan; execution gated). Remaining productive multi-endpoint / WRR execution and fuzz coverage tracked as P2A-OPEN-001 / K8S-P2B.

Evidence collected only from `exyonq-lab-wt-k8s-wedge`.

Note: user inspection path `crates/exyonq-config-ir/` does **not** exist; package `exyonq-config-ir` is at `config/ir/`.

---

## Findings matrix

| FINDING_ID | FILE | SYMBOL | CURRENT_BEHAVIOR | LIMITATION | CONTRACT_IMPACT | K8S_RELEVANCE | NON_K8S_RELEVANCE | ACTION_REQUIRED |
|------------|------|--------|------------------|------------|-----------------|---------------|-------------------|-----------------|
| F01 | `config/ir/src/lib.rs` | `UpstreamConfig` | `name` + single `target` + `timeout_ms` | no endpoint set/weights/policy | blocks Contracts 2–3,7 | High | High | schema evolution |
| F02 | `crates/exyonq-runtime-plan/src/runtime_plan.rs` | `CompiledCluster` / proxy slots | one URI/timeout per upstream | not multi-endpoint | RuntimePlan must gain endpoint set | High | High | P2 compile model |
| F03 | `crates/exyonq-mod-proxy/src/upstream.rs` | `UpstreamDescriptor` | single target metadata | same | DP selection API | High | High | P2 |
| F04 | `config/ir/src/lib.rs` | `validate_upstream_target` | only `http` scheme | no upstream TLS | Contract 9/upstream TLS | High | High | capability flag later |
| F05 | `config/schema/src/lib.rs` | `timeout_ms >= 1` | schema min 1 | TOML path may allow 0 | Contract 7 defaults/reject | Med | Med | align validate |
| F06 | `config/ir/src/lib.rs` | `ServerConfig` | one `listen` string | weak typed listener model | listener contract later | Med | Med | extend carefully |
| F07 | `crates/exyonq-runtime-plan/src/runtime_plan.rs` | `CompiledRouteTable` | global route table id 0 | per-server routes underused | route attachment clarity | Med | Med | design note |
| F08 | `config/ir/src/lib.rs` | `TlsConfig` | cert+key paths | single identity | Contract 9 cert set | High | High | breaking/additive schema |
| F09 | `crates/exyonq-mod-tls/src/lib.rs` | `load_rustls_config` | `with_single_cert` | no multi-SNI | Contract 9 | High | High | P2 TLS |
| F10 | `core/src/reload/mod.rs` | `reload_tls_acceptor` | TLS from `servers[0]` only | other servers ignored | multi-cert programming | High | High | fix in P2 |
| F11 | `crates/exyonq-mod-tls/src/lib.rs` | `load_private_key` | PKCS#8 only | PKCS#1/SEC1 rejected | provider docs | Med | Med | keep unless authorized |
| F12 | `config/ir/src/lib.rs` | `RouteMatch` | path + optional host | no method/header/query | Gateway matches later | High | Med | extend IR |
| F13 | `crates/exyonq-runtime-plan/src/router.rs` | `RouteIndex` | longest path; `starts_with` | weak prefix boundaries | correctness | Med | Med | document/fix |
| F14 | `config/ir/src/lib.rs` | `validate_route_actions` | exactly one action | no compose filters | filters later | Med | Med | evolve actions |
| F15 | `config/merge/src/lib.rs` | `apply_discovery` | `endpoints.first()` | drops remaining endpoints | Contract 2 | **Critical** | High | replace with endpoint set |
| F16 | `crates/exyonq-discovery-runtime/src/file_overlay.rs` | `apply_file_overlay` | fail-open on bad JSON | typed apply errors missing | Contract 11/16 | High | High | fail policy design |
| F17 | `core/src/reload/mod.rs` | `reload_from_path` | gen++ / state swap | TLS before state publish | Contract 10/11 atomicity | High | High | harden ordering |
| F18 | `crates/exyonq-runtime-plan/src/runtime_plan.rs` | `SnapshotFingerprint` | IR hash + module flags | producer/schema version limited | Contract 10/12 | High | High | extend metadata |
| F19 | `crates/exyonq-ops-runtime/src/control_socket.rs` | `parse_command_line` | reload/status/drain/shutdown | no VALIDATE/STAGE/APPLY snapshot API | Contract 11 | High | High | extend ops contract |
| F20 | `cli/exyonqctl/src/main.rs` | `Command` | config lint/test/migrate nginx | no endpoint-set tools yet | migrators/panels | Med | High | later CLI |
| F21 | `config/ir/src/lib.rs` | `AppConfig::parse_str` | validate on construct | no separate validate() | Apply VALIDATE step | Med | Med | expose validate |
| F22 | `config/schema/src/lib.rs` | `ir_json_schema` | partial schema | drift vs IR features | compatibility | Med | High | schema sync |
| F23 | `config/ir/src/error.rs` | `ConfigError` | rich config errors | ops/proxy/tls inconsistent | Contract 16 | Med | High | unify codes |
| F24 | `config/merge/src/product_profile.rs` | comments | no H2/H3 upstream claim; limited retry | pins product limits | Contracts 3/8 | Med | High | keep explicit |
| F25 | `scripts/smoke/p13a-tls-sni.sh` | `SUPPORT_LIMIT=SINGLE_CERT_SNI` | documents single cert SNI | multi-SNI absent | Contract 9 | High | High | P2 |
| F26 | `core/tests/proxy_request_body_limit_test.rs` | comment | no HTTP retry implementation | Contract 8 design-only | retries | Med | High | P2+ |
| F27 | PATH | `crates/exyonq-config-ir` | missing directory | package is `config/ir` | docs clarity | Low | Low | name mapping only |

---

## Config IR gap vs target contracts

| CONTRACT_AREA | CURRENT_TYPE_OR_SYMBOL | CURRENT_CAPABILITY | TARGET_CAPABILITY | BREAKING_CHANGE_REQUIRED | BACKWARD_COMPATIBILITY | MIGRATION_REQUIRED | IMPLEMENTATION_PHASE | PRODUCT_RISK |
|---------------|------------------------|--------------------|-------------------|--------------------------|------------------------|--------------------|----------------------|--------------|
| Backend identity | `UpstreamConfig.name` | name=identity+pool key | BackendId+source+display | NO if keep `name` as BackendId | keep `[[upstream]] name` | map name→backend_id | P1 design / P2 code | Low |
| Endpoint set | `target: String` | single URL | `endpoints[]` | **YES** if remove `target` | keep `target` as legacy single-endpoint sugar | expand target→one endpoint | P2 | Medium |
| LB policy | none | implicit single | WRR+priority | NO (additive enum) | default RR/WRR | defaults | P2 | Medium |
| Health | none in IR | — | admin only in IR | NO | — | — | P2 runtime | Medium |
| Drain | process drain only | lifecycle tokens | per-endpoint drain | NO additive | — | — | P2+ | Medium |
| Route backend ref | `RouteConfig.upstream: Option<String>` | name ref | BackendId ref (+ optional weights) | NO if string remains id | keep field | — | P2 | Low |
| Timeouts | `timeout_ms` | single aggregate | split timeouts | soft YES for final | deprecate alias | map timeout_ms→total | P2 | Medium |
| Retries | none | absent | explicit policy | NO additive | default attempts=1 | — | design P1; code later | Medium |
| TLS cert set | `TlsConfig` paths | single cert/key files | cert set + providers | soft YES | keep cert/key as single-entry sugar | wrap into set | P2 | High |
| Generation | `RELOAD_GENERATION` + fingerprints | DP counter | hybrid producer+DP | NO additive metadata | keep counter | — | P2/ops | Medium |
| Apply protocol | reload path + socket | reload file | validate/stage/apply/ack | NO additive ops | keep reload | — | ops evolve | Medium |
| Versioning | `config_version` u32 | file format | contract major.minor+caps | likely YES long-term | dual-read | version gate | P1–P2 | Medium |
| Producers | file+discovery overlay | fail-open overlay | single authoritative producer | policy YES | discovery becomes producer-owned | — | P2 | High if ignored |
| Obs identity | route label strings | route labels exist | bounded tokens | policy YES | cap cardinality | hash/registry | P2 | Medium |
| Limits | ad hoc | body 32MiB etc | table of hard max | NO additive | — | — | P2 | Medium |
| Failure model | mixed anyhow/thiserror | uneven | structured codes | NO additive | map existing | — | P2 | Medium |

### Explicit answers

```text
EXTENSIBLE_WITHOUT_BREAKING =
  add endpoints[] alongside target sugar;
  add timeout split fields while keeping timeout_ms alias;
  add lb_policy enum defaulting to single/wrr;
  add cert_set while keeping cert/key sugar;
  add producer/schema metadata

REQUIRES_NEW_SCHEMA_VERSION =
  removing target-only model;
  removing single timeout_ms as sole field;
  requiring cert sets without path sugar;
  changing route match semantics incompatibly

TYPES_NOT_TO_REUSE_AS_FINAL =
  UpstreamConfig.target as the only cluster model;
  DiscoveryCluster→first endpoint collapse;
  TlsConfig as the only TLS identity model

LEGACY_TOML_COMPAT =
  continue accepting current [[upstream]] target form as single-endpoint EndpointSet;
  continue cert/key paths as one-element CertificateSet file provider

LEGACY_CONVERSION =
  target URL → Endpoint{id=derived, address/port/protocol from URL, weight=1}
  timeout_ms → total_request_deadline_ms (+ document connect default)

DEPRECATED_FIELDS_CANDIDATES =
  timeout_ms (alias);
  maybe bare target after major bump

STABILIZE_BEFORE_CONTROLLER =
  EndpointSet + BackendId;
  Route→BackendId;
  Cert set references;
  Apply validate/stage/apply/ack + last_good;
  Contract versioning;
  Producer ownership
```

```text
K8S_P1_BREAKING_SCHEMA_CHANGE_REQUIRED = YES
# at least eventually for a clean model; MVP can be additive sugar first (CONDITIONAL)
BREAKING_STRATEGY = PREFER_ADDITIVE_SUGAR_IN_P2_THEN_MAJOR_DEPRECATION

OWNER_ACCEPTANCE = ACCEPT
K8S_P1_STATUS = CLOSED_ACCEPTED
K8S_P1_CLOSED = YES
K8S_P1_CONFIG_IR_READINESS = ACCEPTED_FOR_IMPLEMENTATION_ADMISSION
K8S_P2_READY_TO_ADMIT = YES_FOR_SEPARATE_SCOPE_PREPARATION
K8S_P2_OPENED = NO
K8S_P3_OPENED = NO
K8S_P6_OPENED = NO
IMPLEMENTATION_AUTHORIZED = NO
```
