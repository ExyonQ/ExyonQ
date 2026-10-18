# V044 — Specialized proxy dataplane: contracts & integration design

```text
WIP = V044_SPECIALIZED_PROXY_DATAPLANE_CONTRACT_AND_INTEGRATION_DESIGN
MODE = READ_ONLY_ARCHITECTURE_CONTRACT_AND_INCREMENTAL_MIGRATION_DESIGN
STATUS = COMPLETE
CASE = SD-B
PARENT_CASE = EP-C
STRATEGIC_PARENT = GA-B
PRODUCT_MUTATION = NO
PROTOTYPE_INTEGRATION = NO
PRODUCT_CRATE_CREATION = NO
SOURCE_EXYONQ_HEAD = 0bc2b973e5ff62b2316fbc7c3f002d673c65d791
SOURCE_EXYONQ_TREE = b1b7659b4d3bb8745ea0ed42a8de7d424f523767
```

Governance: [`../governance/v044-specialized-proxy-dataplane-integration-design.md`](../governance/v044-specialized-proxy-dataplane-integration-design.md)
Performance budget: [`../performance/v044-specialized-proxy-dataplane-integration-budget.md`](../performance/v044-specialized-proxy-dataplane-integration-budget.md)
Evidence: `.exyonq-local/evidence/specialized-proxy-dataplane-design/20260827-200623/`

Parents: EP-C · GA-B · PXDP-P0 (P0-A) · PXDP-B · DP-C

---

## 1. Primary answer

```text
HOW_CAN_EXYONQ_ADOPT_A_SPECIALIZED_PROXY_DATAPLANE_WHILE_PRESERVING_EXISTING_PRODUCT_LOGIC
  AND_WITHOUT_REWRITING_THE_PRODUCT =

  Keep product/control SSOT on Tokio (config, reload, routing tables, WAF policy
  compilers, modules, observability exporters, control API, Cap067, FastCGI).

  Publish immutable GenerationProjection + compiled policy artifacts into
  shard-local mio TPC dataplanes.

  Dataplane owns: H1 wire parse, connection/upstream ownership, timers,
  bounded relay, and *execution* of already-compiled admission decisions.

  Each request builds a fresh ProxyExecutionPlan (adapt PXDP-P0 ProxyRequestPlan
  semantics) after parse + local lookup against frozen generation — never store
  request policy on the long-lived connection.

  Unsupported features fall back to legacy Hyper path *before* specialized
  side effects consume ambiguous socket state.

  Do NOT copy the 925-LOC prototype into product; copy the *geometry*.
```

```text
CASE = SD-B
CASE_NAME = FEASIBLE_WITH_BOUNDED_PRODUCT_CONTRACT_CHANGES
```

Bounded changes = new internal projection/admission contracts + feature gate +
compiled WAF/rate shard views — **not** public module API rewrite, not Cap067
touch, not Tokio/Hyper removal.

---

## 2. Target architecture (planes)

```text
┌── ExyonQ Product Plane (Tokio) ─────────────────────────────┐
│ config / reload / generations                               │
│ routing SSOT · WAF policy SSOT · rate policy SSOT           │
│ module API · observability exporters · control API          │
│ FastCGI · Cap067 static/sendfile                            │
│ compile → GenerationProjection + CompiledPolicyArtifacts    │
└───────────────────────────┬─────────────────────────────────┘
                            │ atomic publish (no request Mutex)
                            ▼
┌── Specialized Proxy Dataplane (mio TPC shards) ─────────────┐
│ SO_REUSEPORT / shard-local accept                           │
│ CLIENT_CONNECTION_OWNER = ONE_SHARD_FOR_LIFETIME            │
│ UPSTREAM_OWNER = SHARD_LOCAL (no global pool lock)          │
│ H1 parse + ProxyExecutionPlan (fresh / request)             │
│ local WAF/rate *execution* on compiled artifacts            │
│ direct bounded relay · timers · disposition                 │
└─────────────────────────────────────────────────────────────┘
```

```text
PRODUCT_LOGIC_SSOT          = product plane
PRODUCT_LOGIC_EXECUTION_LOCATION = may be shard-local on immutable artifacts
DOUBLE_PARSE                = REJECT_PREFERRED
NO_UPSTREAM_SIDE_EFFECT_BEFORE_REQUIRED_SECURITY_POLICY_PASS = YES
```

---

## 3. ADR-021 production stack recommendation

| ID | Option | Verdict |
|----|--------|---------|
| A | Extend 925-LOC prototype literally into ExyonQ | **REJECT** as product source — geometry yes, code copy no |
| **B** | **mio TPC shell + mature httparse (+ socket2); harden H1** | **RECOMMENDED** |
| C | Sharded current-thread Tokio | Weaker EP-C falsifier; optional later experiment |
| D | Hyper low-level inside shard | Risks reintroducing Hyper geometry costs |
| E | Retain PXDP Hyper shell | Perf FAIL (PR-C); contracts/tests reusable |
| F | Full custom HTTP stack | High risk; not first move |

```text
RECOMMENDED_PRODUCTION_STACK = B
VALIDATED_GEOMETRY = THREAD_PER_CORE_SHARD_LOCAL_EVENT_LOOP (EP-C)
PROTOTYPE_SOURCE = REFERENCE_ONLY
```

---

## 4. Product ↔ dataplane contract: `ProxyExecutionPlan`

Name deliberately distinct from docs-only PXDP `ProxyRequestPlan` to avoid
implying Hyper-shell identity — **semantics reuse PXDP-P0 invariants**.

### 4.1 Lifetime

```text
ONE_REQUEST = ONE_FRESH_ProxyExecutionPlan
CLIENT_CONNECTION may outlive many plans
NEVER store request policy on connection for keepalive reuse
```

### 4.2 Minimum fields (v1 GET known-length)

| Field | Purpose |
|-------|---------|
| `generation_id` | Freshness; recheck each request |
| `method` | GET-only in first slice |
| `path_query` | Normalized target |
| `authority_key` / host class | Routing input already resolved |
| `route_id` / `cluster_id` | Frozen route outcome |
| `upstream_endpoint` | Shard-local connect/lease key |
| `header_projection` | Hop-by-hop stripped; Host rewrite for upstream |
| `waf_admission` | Allow / reject(+status) — **computed before upstream** |
| `rate_admission` | Allow / reject — if specialized path admits rate |
| `timeout_policy` | connect / header / body / idle bounds |
| `retry_policy` | Pre-commit only budget (view) |
| `framing_hints` | CL expected; disposition defaults |
| `obs_correlation` | Compact IDs / sample bit |
| `capability_flags` | Feature gates (body, WS, …) |
| `fallback_reason` | If built only to divert to legacy — prefer divert *before* plan |

**Forbidden in plan:** mutable `ServerState`, `Arc<RwLock<…>>`, live module dyn objects, Hyper types, Tokio `Handle` as hot dependency.

### 4.3 Build order (security)

```text
ACCEPT → READ_HEADERS (dataplane parse)
  → NORMALIZE
  → LOOKUP GenerationProjection (atomic load)
  → ROUTE (immutable table)
  → WAF_EXECUTE (compiled artifacts)     // MUST PASS before upstream
  → RATE_EXECUTE (if on specialized path)
  → BUILD ProxyExecutionPlan
  → UPSTREAM_CONNECT/WRITE
  → RELAY
```

```text
NO_UPSTREAM_SIDE_EFFECT_BEFORE_REQUIRED_SECURITY_POLICY_PASS = YES
```

---

## 5. Request parsing ownership

| Option | Verdict |
|--------|---------|
| **A dataplane owns wire parse → NormalizedRequestView** | **SELECTED** for specialized path |
| B Hyper parses then transfers | Double work; handoff tax |
| C Shared parser both paths | Acceptable long-term if one representation |
| D Other | — |

```text
REQUEST_PARSE_OWNER (specialized path) = DATAPLANE
LEGACY_PATH_PARSE = HYPER (unchanged)
DOUBLE_PARSE = REJECT_PREFERRED
```

Fallback must occur on **eligibility** (method/path/features) using a *minimal*
prefix peek or first-line parse that does not leave the socket in an ambiguous
half-consumed specialized state — or use separate listeners / SO_REUSEPORT
admission that never enters specialized parse for unsupported classes.

---

## 6. H1 required feature matrix (implementation gate; not built now)

| Feature | First product slice | Later | Notes |
|---------|---------------------|-------|-------|
| request-line + headers | REQUIRED | — | httparse |
| Host | REQUIRED | — | |
| Content-Length request == 0 / absent | REQUIRED | — | reject body |
| Transfer-Encoding request | REJECT | later | smuggling |
| response CL known-length | REQUIRED | — | |
| response chunked | FALLBACK | REQUIRED | |
| trailers | FALLBACK | later | |
| client keepalive | REQUIRED | — | |
| upstream keepalive shard-local | REQUIRED | — | |
| HEAD / 204 / 304 | FALLBACK | REQUIRED | |
| 1xx / Expect: 100 | FALLBACK | later | |
| request body stream | HOLD | SDP body phase | |
| upgrades / WS / CONNECT | FALLBACK | tunnel phase | |
| pipeline | REJECT or serialize | policy | |
| CL/TE defenses | REQUIRED | — | |
| header size limits | REQUIRED | — | |
| obs-fold | REJECT | policy | |
| hop-by-hop strip | REQUIRED | — | |
| premature EOF / length mismatch | REQUIRED fail-closed | — | |

---

## 7. Connection ownership & accept

```text
CLIENT_CONNECTION_OWNER = ONE_SHARD_FOR_LIFETIME
UPSTREAM_CONNECTION_OWNER = SHARD_LOCAL
CROSS_SHARD_REQUEST_HANDOFF = 0
GLOBAL_UPSTREAM_POOL = NO
GLOBAL_CONNECTION_LOCK = NO
```

| Accept model | Linux | macOS | Windows |
|--------------|-------|-------|---------|
| **A SO_REUSEPORT per shard** | **Preferred** (EP-C) | Supported via mio | Later / different |
| B single acceptor + one-time dispatch | Portable fallback | OK | OK |
| C platform-specific | Optional | — | IOCP readiness |

```text
SHARDING_MODEL = EXPLICIT_SHARD_COUNT_FROM_CPUSET_QUOTA
  shards = f(available_cpus, numa_nodes, cpuset, quota)
  NO hardcoded global "4"
```

EP-C used 4 shards on 8 CPUs as **campaign geometry only**.

---

## 8. Global parallelism budget (conceptual)

```text
ExyonQParallelismBudget (design SSOT; not implemented)
  proxy_shards
  accept (if separate)
  Cap067_epoll_pool
  tokio_worker_threads (control/services)
  blocking_pool
  FastCGI / background
```

Goal: prevent accidental oversubscription; allow specialized paths to scale
independently. Not aesthetic unification.

NUMA: prefer shard placement and upstream pools local to node; generation
snapshots may be shared read-mostly across nodes (accept cross-NUMA reads of
immutable data). No hard pin by default (shared hosts).

---

## 9. Generation publication

```text
reload (Tokio) → compile GenerationProjection + CompiledPolicyArtifacts
              → atomic ArcSwap / RCU publish
shards: per request atomic load; pin Arc for plan lifetime
old generation live until in-flight plans drop
```

Requirements: no request-hot Mutex; safe Rust; no use-after-free; Cap067
generation compare patterns remain informative precedent.

---

## 10. Routing / WAF / rate / modules

### Routing

```text
ROUTING_INTEGRATION = SHARED_IMMUTABLE_SNAPSHOT + PRECOMPILED_LOOKUP
SSOT = existing route tables / host ranking
NO route logic duplication inside dataplane
```

### WAF (critical)

Per-request Tokio RPC for WAF **would destroy EP-C advantage**.

```text
WAF_INTEGRATION = HYBRID
  SSOT / policy compile / reload = product plane
  Per-request execute = shard-local on CompiledWafArtifacts
  Enforce ordering = before upstream (unchanged Cap015/wire truth)
```

If a WAF mode cannot be compiled to shard-local execution → **FALLBACK** to
legacy path for that request class (eligibility), not cross-runtime call.

### Rate limiting

Config `RateLimitModule` today is Hyper-pipeline / LEGACY for wire.

```text
RATE_LIMIT_INTEGRATION =
  specialized path: shard-partitioned or atomic shared counters from compiled policy
  OR FALLBACK until semantics preserved
  WAF abuse rate facet: include in CompiledWafArtifacts when on specialized path
```

Do not silently weaken global quotas.

### Module API

| Hook class | Specialized path |
|------------|------------------|
| HOT_SYNCHRONOUS (`on_request`/`on_route` that must run) | Compile to flags / deny / FALLBACK |
| RESPONSE_MUTATING (compression, …) | FALLBACK initially |
| BODY_MUTATING | FALLBACK / later |
| CONTROL_PLANE | Stay Tokio |
| Metrics counters | Prefer dataplane counters + batch |

```text
MODULE_API_INTEGRATION = PRESERVE_API + ADAPT_ADMISSION
  First slice: modules_enabled / response mutators → LEGACY fallback
  Do NOT redesign stable module API in SDP-P1
```

---

## 11. Observability / logging

```text
OBSERVABILITY_INTEGRATION = PRESERVE_WITH_DATAPLANE_ADAPTER
  per-shard counters + bounded event rings
  Tokio plane batches → OTLP / metrics / syslog / journald / files
  NO heavy exporter on shard hot path
  NO synchronous global logger lock per request
```

Overload: bounded queue → sample/drop with metric (align with existing
governance; document exact policy in implementation WIP).

---

## 12. Retry / timeouts / backpressure

| Concern | Design |
|---------|--------|
| Retry | Pre-commit only; attempt in plan; no retry after downstream commit (PXDP-P0) |
| Timeouts | Shard timer wheel / mio poll deadline — **not** per-request Tokio timers |
| Backpressure | Bounded buffers; disable upstream read on downstream high-water; no whole-body buffer |

---

## 13. Protocol futures (compatibility, not scope)

| Domain | Relation |
|--------|----------|
| Request body | HOLD; architecture must allow later streaming + WAF body gate |
| Chunked / SSE / long streams | Framing state machine extension |
| WS / CONNECT | Shard ownership advantageous; separate tunnel phase |
| H2 | **Separate** protocol dataplane (Hyper or specialized) — do not force into H1 loop |
| H3/QUIC | Provider-owned; hybrid dispatch boundary |
| FastCGI | **PRESERVE**; migration NOT_REQUIRED initially |
| Cap067 / static | **PRESERVE**; specialized proxy must not replace; cheap dispatch remain |
| Control API | Hyper/Tokio **PRESERVE** |

---

## 14. State machine (connection / request)

```text
CLIENT_READING_HEADERS
PLAN_READY
UPSTREAM_CONNECTING
UPSTREAM_WRITING_REQUEST
UPSTREAM_READING_HEADERS
RELAYING_BODY
DOWNSTREAM_FLUSH
KEEPALIVE_IDLE
CLOSING
ERROR
```

Owner: shard session object (EP-C session analogue). Plan attached only in
`PLAN_READY`…terminal; cleared before `KEEPALIVE_IDLE`.

---

## 15. Fallback boundary

```text
SUPPORTED_SPECIALIZED_REQUEST → dataplane
UNSUPPORTED → legacy Hyper/wire path
```

**Hard rule:** no same-socket late fallback after specialized parser has consumed
bytes unless a proven rewind/ownership handoff exists (default: **forbid**).

Eligibility before specialized consume:

- method ∈ {GET} (first slice)
- path class ∈ `/api/` (or configured proxy class)
- no TE, no request body, no Upgrade
- no response-mutating modules required
- WAF mode compilable to shard artifacts
- feature flag / canary admits traffic

---

## 16. Canary / feature flag (design only)

```text
default OFF
env/config capability gate
optional route allowlist
optional percentage if safe (same eligibility)
side-by-side metrics vs legacy
```

---

## 17. PXDP relation

| Asset | Future |
|-------|--------|
| PXDP Hyper execution code | **RETIRE / SUPERSEDE** for competitive H1 path (after SDP proves) |
| PXDP-P0 contracts (fresh plan, WAF order, retry, framing, disposition) | **ADAPT → ProxyExecutionPlan / SDP shell** |
| PXDP architecture design (worker-local shell) | **ADAPT geometry** from Tokio-task to mio-shard |
| PXDP tests / oracles (P0–P5 intent) | **PRESERVE / ADAPT** as contract tests |
| PXDP-P6 / body-relay | **HOLD** until SDP body phase |

Sunk cost does not preserve Hyper PXDP execution.

---

## 18. Test asset reuse (contract tests)

Carry forward as SDP gates:

- fresh plan per request / generation freshness
- WAF before upstream
- Cap015 keepalive WAF re-entry
- backpressure / no unbounded buffer
- commit boundary / retry pre-commit
- framing CL/TE / smuggling rejects
- keepalive boundaries
- error mapping (502/504/…)

---

## 19. Migration phases (recommended)

| Phase | Content | Gate |
|-------|---------|------|
| **SDP-D0** | This design (contracts) | SD-A/B |
| **SDP-D1** | Shadow: plan build + metrics only (optional) | correctness |
| **SDP-P1** | Product slice: GET CL upstream, flag OFF default | ≥15% RPS vs current ExyonQ (≥20% preferred) |
| **SDP-P2** | Generation + compiled WAF parity + keepalive | security + perf |
| **SDP-P3** | Observability adapters + canary | ops |
| **SDP-P4** | Chunked / streaming response | |
| **SDP-P5** | Request bodies (unblocks HOLD body relay under SDP) | |
| **SDP-P6** | Module admission expansion | |
| Later | WS/CONNECT; never force H2/H3 into H1 loop | |

```text
FIRST_PRODUCT_SLICE = SDP-P1
  GET + known-length + /api/ class + compiled WAF allow/deny
  + generation freshness + shard-local upstream KA + client KA
  + minimal counters; modules that mutate → fallback
```

```text
PRODUCT_IMPLEMENTATION_AUTHORIZED = NO  # until owner opens SDP-P1
```

---

## 20. Component preserve matrix

| Component | Action |
|-----------|--------|
| routing | **PRESERVE** |
| WAF policy SSOT | **PRESERVE**; execution location **ADAPT** (compiled) |
| module API | **PRESERVE** / admission **ADAPT** |
| observability exporters | **PRESERVE_WITH_DATAPLANE_ADAPTER** |
| reload | **PRESERVE** (+ projection publish) |
| Cap067 | **PRESERVE** |
| FastCGI | **PRESERVE** |
| control API | **PRESERVE** |
| Tokio | **PRESERVE** (substrate) |
| Hyper | **ROLE_SCOPED** (control/H2/legacy) |
| PXDP execution | **RETIRE** after SDP success |
| PXDP semantic tests | **PRESERVE / ADAPT** |

### Must NOT rewrite for SDP adoption

- Cap067 / sendfile FSM
- Routing algorithm SSOT
- WAF policy language / engine SSOT (compile, don’t fork)
- Module trait public API (admission only)
- Control API / ops socket
- FastCGI runtime (initially)
- Entire Tokio process model
- Config IR wholesale

---

## 21. Risk ledger (ranked)

| Risk | Sev | Note |
|------|-----|------|
| H1 protocol / smuggling burden | High | Matrix + fuzz gates |
| WAF locality vs advantage | High | Compiled artifacts mandatory |
| Module compatibility | High | Fallback eligibility |
| Security regressions | High | Ordering + tests |
| Reload/generation publish bugs | Med–High | Arc lifetime |
| Observability overhead | Med | Budget § |
| Portability (Win later) | Med | mio abstraction |
| H2/H3 divergence | Med | Explicit multi-dataplane |
| Maintenance of H1 engine | Med | Prefer httparse+hardening over NIH |
| Testing burden | Med | Reuse PXDP oracles |

```text
SAFE_RUST_FEASIBLE = YES
  Unsafe only if later proven at mio/socket boundary with documented invariant
```

---

## 22. Dependencies (proposed; no Cargo.toml change)

| Crate | Role | License (typical) | Note |
|-------|------|-------------------|------|
| mio | readiness | MIT | EP-C proven |
| httparse | H1 headers | MIT/Apache-2.0 | keep |
| socket2 | REUSEPORT / nodelay | MIT/Apache-2.0 | keep |

No new deps authorized in this WIP. Alternatives (hyper h1 types only, http crate) deferred to SDP-P1 admission.

---

## 23. Portability model

```text
PORTABILITY_MODEL = mio readiness abstraction
  Linux = epoll
  macOS = kqueue (iteration)
  Windows = future IOCP via mio
Platform specialization under trait OK; no Linux-only types in product IR
```

---

## 24. Error model

Compact hot-path enum (adapt PXDP-P0 `ProxyError` taxonomy): protocol, policy reject, upstream connect/timeout, generation invalidated, canceled, internal. Map to existing access/log outcomes without heap per event on success path.

---

## 25. CASE discrimination

| Case | Why not / why |
|------|----------------|
| SD-A | Almost — but compiled WAF/rate/module admission needs **bounded** new contracts |
| **SD-B** | **Selected** |
| SD-C | Module/security not blocking if FALLBACK eligibility holds |
| SD-D | EP-C +20% preferred gate leaves ~10–12 µs budget — plausible if WAF stays local |
| SD-E | Boundaries clean enough (Cap067 / control / FastCGI preserved) |
| SD-F | Evidence sufficient (EP-C + PXDP-P0 + live path map) |

---

## 26. Explicit non-claims

- No prototype integration, product crate, or dependency change in this WIP.
- EP-C validates **geometry**, not a mandate to merge experiment sources.
- First product slice may be slower than bare prototype; gate is vs **current ExyonQ**, not vs OLS.
