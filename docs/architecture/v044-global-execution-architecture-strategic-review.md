# V044 — Global execution architecture strategic review

```text
WIP = V044_GLOBAL_EXECUTION_ARCHITECTURE_STRATEGIC_REVIEW
MODE = READ_ONLY_WHOLE_PRODUCT_EXECUTION_ARCHITECTURE_REVIEW
STATUS = COMPLETE
CASE = GA-B
PRODUCT_MUTATION = NO
ARCHITECTURE_MUTATION = NO
HARNESS_MUTATION = NO
SOURCE_HEAD = 0bc2b973e5ff62b2316fbc7c3f002d673c65d791
SOURCE_TREE = b1b7659b4d3bb8745ea0ed42a8de7d424f523767
```

Governance: [`../governance/v044-global-execution-architecture-strategic-review.md`](../governance/v044-global-execution-architecture-strategic-review.md)  
Performance: [`../performance/v044-global-execution-architecture-strategic-review.md`](../performance/v044-global-execution-architecture-strategic-review.md)  
Evidence: `.exyonq-local/evidence/global-execution-architecture-review/20260827-181625/`

Parents (closed): PR-C · RC-B · UG-C · SW-C · MC-B · WG-B/EC-A · RT-C · CP-C · DP-C · PXDP-B

---

## 1. Primary question

```text
IS_THE_CURRENT_HYPER_TOKIO_AND_EXECUTION_MODEL_THE_RIGHT_ARCHITECTURE_FOR_EXYONQ_AS_A_WHOLE_PRODUCT =
  NO_AS_A_UNIVERSAL_HOT_PATH_MODEL
  YES_AS_PROCESS_SUBSTRATE_AND_CONTROL_PLANE
  YES_WITH_CAP067_FOR_STATIC_SENDFILE
  NO_FOR_COMPETITIVE_PROXY_DATAPLANE_WITHOUT_SPECIALIZATION
```

Distinguish:

| Layer | Assessment |
|-------|------------|
| **Product architecture** (routing, WAF, generations, modules, config IR) | **ACCEPTABLE** — preserve |
| **Execution architecture** (who owns sockets, threads, tasks) | **MIXED** — Cap067 good; proxy poor |
| **Protocol stack** (Hyper H1/H2, H3 providers) | **ROLE-SCOPED** — not one answer |
| **Runtime** (one Tokio multi_thread) | **KEEP** as process/control substrate |
| **Hot-path implementation** (proxy Hyper client + task stacking) | **NEEDS SPECIALIZATION** |

Whole-product rewrite (**GA-E**) is **REJECTED**. Cap067 P3 competitiveness + CP-C refute “architecture globally broken.”

---

## 2. Claim boundary (do not over-generalize)

| Claim | Status |
|-------|--------|
| Tokio is bad for all ExyonQ | **FALSE** — control, reload, FastCGI bridge, H3 providers fit |
| Hyper is bad for all ExyonQ | **FALSE** — control API, H2 auto path; proxy *hot* role is the issue |
| Rust async is bad | **FALSE** — evidence is ownership/geometry, not language |
| Static path is bad | **FALSE** — P2/P3 strong; P1 bounded residual |
| Cap067 is bad / P4 poison | **FALSE** — CP-C NOT_MATERIAL |

---

## 3. Domain inventory (EXEC-01 … EXEC-16)

### EXEC-01 — HTTP accept / connection ownership

| Field | Value |
|-------|-------|
| CURRENT_EXECUTION_MODEL | Tokio `accept_loop` tasks (default); optional Linux epoll listen / io_uring |
| TOKIO_DEPENDENCY | CORE |
| HYPER_DEPENDENCY | INCIDENTAL (post-accept) |
| THREAD_MODEL | Tokio tasks on RT-01; Cap067 OS threads for keepalive divert |
| CONNECTION_OWNERSHIP | Accept task → Hyper conn or Cap067 session or wire |
| CROSS_THREAD_HANDOFFS | Accept → Cap067 / Hyper handoff seams |
| ARCHITECTURE_FIT | ACCEPTABLE |
| REASON | Multi-mode accept is coherent; proxy path later pays handoff tax |

### EXEC-02 — HTTP/1 request parsing / lifecycle

| Field | Value |
|-------|-------|
| CURRENT_EXECUTION_MODEL | Hyper `auto::Builder` server **or** Cap067/wire byte parse |
| TOKIO_DEPENDENCY | CORE (Hyper path) / OPTIONAL (Cap067) |
| HYPER_DEPENDENCY | CORE on Hyper path; NONE on Cap067 sendfile steady state |
| ARCHITECTURE_FIT | QUESTIONABLE on proxy (task+driver); GOOD on Cap067 |
| REASON | Dual parsers by design; proxy stays in Hyper driver class (MC-B) |

### EXEC-03 — Static small-file (P1)

| Field | Value |
|-------|-------|
| CURRENT_EXECUTION_MODEL | Cap067 / wire + possible Hyper fallback |
| TOKIO_DEPENDENCY | OPTIONAL (first request / handoff) |
| HYPER_DEPENDENCY | OPTIONAL |
| KNOWN_PERFORMANCE_EVIDENCE | P1 −8.8% RPS vs NGINX; **CPU win** −19.3% (DP-C) |
| ARCHITECTURE_FIT | ACCEPTABLE / QUESTIONABLE (RPS) |
| REASON | Fixed-cost sensitivity; not rewrite justification |

### EXEC-04 — Large static / sendfile / Cap067

| Field | Value |
|-------|-------|
| CURRENT_EXECUTION_MODEL | OS epoll FSM + sendfile (`exyonq-platform-linux` + `exyonq-mod-static`) |
| TOKIO_DEPENDENCY | INCIDENTAL (lazy start / handoff) |
| HYPER_DEPENDENCY | NONE on steady sendfile |
| THREAD_MODEL | Cap067 OS pool (inherits `tokio-rt-worker` name — RT-C) |
| KNOWN_PERFORMANCE_EVIDENCE | P3 **COMPETITIVE_WIN**; P2 +15.3% RPS |
| ARCHITECTURE_FIT | **GOOD** |
| REASON | Preserve as independent successful dataplane |

### EXEC-05 — Reverse proxy

| Field | Value |
|-------|-------|
| CURRENT_EXECUTION_MODEL | Tokio + Hyper server/wire + Hyper legacy client (PXDP P4) |
| TOKIO_DEPENDENCY | CORE |
| HYPER_DEPENDENCY | CORE (server + client) |
| KNOWN_PERFORMANCE_EVIDENCE | PR-C: ~76k vs NGINX ~96k / OLS ~100k; PXDP +4% only; MC-B: minimal Hyper+Tokio −32% vs NGINX, +15% vs ExyonQ |
| ARCHITECTURE_FIT | **POOR** |
| REASON | Substrate ctx class (~100+) vs NGINX (~14) **and** ExyonQ integration tax |

### EXEC-06 — FastCGI / PHP-FPM

| Field | Value |
|-------|-------|
| CURRENT_EXECUTION_MODEL | Live module + ConnPool; `spawn_blocking` transport on RT-01 |
| TOKIO_DEPENDENCY | CORE |
| HYPER_DEPENDENCY | INCIDENTAL (request types via handler) |
| ARCHITECTURE_FIT | ACCEPTABLE (competitive **UNKNOWN**) |
| REASON | Do **not** auto-migrate with proxy redesign |

### EXEC-07 — WAF

| Field | Value |
|-------|-------|
| CURRENT_EXECUTION_MODEL | Wire admit + Hyper header/body gates; ARCH-002 skip when inactive |
| TOKIO_DEPENDENCY | OPTIONAL/CORE by path |
| HYPER_DEPENDENCY | OPTIONAL |
| ARCHITECTURE_FIT | ACCEPTABLE |
| REASON | Preserve ordering (before upstream effects); adapter if proxy shell changes |

### EXEC-08 — Routing / rewrite / htaccess

| Field | Value |
|-------|-------|
| CURRENT_EXECUTION_MODEL | Generation / Arc snapshot; htaccess watcher (notify OS thread) |
| TOKIO_DEPENDENCY | OPTIONAL |
| HYPER_DEPENDENCY | NONE |
| ARCHITECTURE_FIT | GOOD |
| REASON | Product contracts; not P4 RPS root |

### EXEC-09 — Observability hot-path

| Field | Value |
|-------|-------|
| CURRENT_EXECUTION_MODEL | Same Tokio RT; OTLP/metrics/logging crates |
| TOKIO_DEPENDENCY | CORE for async exporters |
| ARCHITECTURE_FIT | ACCEPTABLE if kept off dataplane critical section |
| REASON | Decouple asynchronously; do not strip requirements for speed |

### EXEC-10 — Config generation / reload

| Field | Value |
|-------|-------|
| CURRENT_EXECUTION_MODEL | Reload runtime tasks on RT-01; immutable generations |
| TOKIO_DEPENDENCY | CORE |
| HYPER_DEPENDENCY | NONE |
| ARCHITECTURE_FIT | GOOD |
| CONTROL_DATA_PLANE_SEPARATION | PARTIAL (same RT, different tasks) |

### EXEC-11 — Long-lived streaming / SSE

| Field | Value |
|-------|-------|
| CURRENT_EXECUTION_MODEL | Hyper/Tokio body streaming |
| ARCHITECTURE_FIT | UNKNOWN / HOLD |
| REASON | No competitive campaign in this review |

### EXEC-12 — WebSocket / upgrade

| Field | Value |
|-------|-------|
| CURRENT_EXECUTION_MODEL | Hyper upgrade + proxy client |
| ARCHITECTURE_FIT | UNKNOWN / HOLD_IF_EXECUTION_DEPENDENT |

### EXEC-13 — H2 future

| Field | Value |
|-------|-------|
| CURRENT_EXECUTION_MODEL | Hyper `auto` http2; ALPN forces Hyper (not Cap067) |
| ARCHITECTURE_FIT | ACCEPTABLE for Hyper role; HOLD expansion on unsettled proxy substrate |
| REASON | Multiplexing favors connection-local ownership — aligns with specialized shell later |

### EXEC-14 — H3 / QUIC

| Field | Value |
|-------|-------|
| CURRENT_EXECUTION_MODEL | Provider crates on same `Handle` (s2n/quiche) |
| TOKIO_DEPENDENCY | CORE (Handle) / spawn_blocking for some providers |
| HYPER_DEPENDENCY | INCIDENTAL (response types) |
| ARCHITECTURE_FIT | ACCEPTABLE as provider-owned; not forced through proxy shell |
| REASON | QUIC timers/events remain provider-local |

### EXEC-15 — Control HTTP API

| Field | Value |
|-------|-------|
| CURRENT_EXECUTION_MODEL | Hyper server on RT-01 (`exyonq-control-api`) |
| ARCHITECTURE_FIT | **GOOD** |
| REASON | Off competitive hot path — keep Hyper |

### EXEC-16 — Background / administrative

| Field | Value |
|-------|-------|
| CURRENT_EXECUTION_MODEL | Ops Unix socket (Tokio, not Hyper); Redis OS thread when used; notify |
| ARCHITECTURE_FIT | GOOD |

---

## 4. Hyper role map

| Use | Hot path? | Cost material? | Change independently? |
|-----|-----------|----------------|----------------------|
| Server cleartext/TLS Hyper | YES (non-Cap067) | YES on proxy/small req | PARTIAL (per path) |
| Cap067 static sendfile | NO (steady) | NO | Preserve Cap067 |
| Proxy upstream client | YES | YES (MC-B class) | YES — primary specialization target |
| PXDP wire write | YES | YES | YES with shell |
| Control HTTP API | NO | NO | Keep |
| H2 via auto Builder | YES when ALPN h2 | UNKNOWN | Role-scoped keep |
| H3 response types | YES when H3 | UNKNOWN | Provider-owned |

```text
HYPER_GLOBAL_ASSESSMENT = ROLE_SCOPED_PARTIAL_REDUCTION_ON_PROXY_HOT_PATH
HYPER_REPLACEMENT_REQUIRED = PARTIAL
```

---

## 5. Tokio role map

| Responsibility | Assessment |
|----------------|------------|
| Sole production Runtime (CLI Builder) | Proven RT-C / DB5 |
| Listener/accept tasks | Keep |
| Socket IO (Hyper paths) | Keep for non-proxy / control |
| Timers / task scheduling | Keep for control & services |
| Proxy dataplane | Constraint (MC-B) — specialize ownership |
| Static Cap067 | Incidental |
| FastCGI spawn_blocking | Acceptable |
| Observability / reload / control | Appropriate |
| H3 providers | Appropriate Handle sharing |

```text
TOKIO_GLOBAL_ASSESSMENT = KEEP_AS_PROCESS_AND_CONTROL_SUBSTRATE
TOKIO_REPLACEMENT_REQUIRED = NO
```

Tokio is a **global substrate**, not a proxy-only bug — but proxy **must not** assume multi_thread work-stealing task geometry is competitive with NGINX event-loop ownership.

---

## 6. Control vs data plane

```text
CONTROL_DATA_PLANE_SEPARATION = PARTIAL
```

Evidence: one Tokio RT hosts accept, proxy, reload, control API, observability (RT-C). Cap067 is already a **separate OS dataplane**. Mixing is acceptable for control; **proxy dataplane should not share scheduling fate with work-stealing tasks at NGINX-class targets**.

---

## 7. Global thread model (RT-C)

Coherent today:

- 1× Tokio multi_thread
- Independent CPU-derived accept / Cap067 sizing
- Optional pools (conn pool inactive on default P4)

**Not** a defect merely because pools are independent. Defect for large hosts: **no global budget** + `MAX_WORKERS=16` clamp → 40-CPU scaling **UNRESOLVED** (do not hardcode 4/8/16 as architecture).

---

## 8. Model comparison (A–H)

| Model | Proxy | Static | Sendfile | FastCGI | WAF | Long conn | H2 | H3 | Win/Mac | Linux | Complexity | Incremental? | Verdict |
|-------|-------|--------|----------|---------|-----|-----------|----|----|---------|-------|------------|--------------|---------|
| **A** Current Hyper+Tokio | POOR | OK | GOOD | OK | OK | OK | OK | OK | OK | OK | Low | — | Status quo; reject as sole proxy strategy |
| **B** Segmented Tokio RTs | ? | OK | GOOD | OK | OK | OK | OK | OK | OK | OK | Med | YES | May help isolation; **unlikely alone** to hit NGINX ctx (MC-B) |
| **C** Current-thread shards | Better? | OK | GOOD | OK | OK | Better locality | Better | Careful | OK | OK | Med–High | YES | Candidate **prototype class** |
| **D** TPC event-loop DP + Tokio CP | Best fit | OK | GOOD | OK | Adapter | Strong | Strong | Provider | Harder | Strong | High | YES | Strong hybrid variant |
| **E** mio/direct epoll DP | Best falsifier | OK | GOOD | Separate | Adapter | Strong | Later | Separate | Port layer | Strong | High | YES | **Recommended prototype** |
| **F** Hybrid (Cap067 + specialized proxy + Tokio CP) | Target | GOOD | GOOD | Keep | Keep | Design | Role Hyper | Provider | Portable CP | DP Linux-first OK | Med–High | **YES** | **RECOMMENDED (GA-B)** |
| **G** Alt HTTP stack/runtime | Speculative | Risk Cap067 | Risk | Risk | Risk | ? | ? | ? | ? | ? | Very high | NO | Not first move |
| **H** Custom protocol engine | Max risk | Max risk | Max risk | — | — | — | — | — | — | — | Extreme | NO | Reject without overwhelming evidence |

```text
RECOMMENDED_EXECUTION_MODEL = MODEL_F_HYBRID
```

---

## 9. Scaling / NUMA / affinity (reasoning only)

| CPUs | Concern |
|------|---------|
| 2–8 | Current model measurable; w4 peaked on 8-CPU P4 (WG-B) — **not** a default auth |
| 16 | Default clamp zone |
| 32–128 | Independent pools ×16 → under-use cores **or** oversubscribe if clamp raised blindly |

NUMA: no locality today for upstream leases / buffers → **cross-NUMA risk** grows with socket count. Prefer **connection-owned shard locality** over hard pinning by default (shared hosts / portability). Soft locality > hard pin unless dedicated appliance profile.

```text
CURRENT_LARGE_CPU_SCALING_RISK = MEDIUM_TO_HIGH_UNRESOLVED
GLOBAL_PARALLELISM_MODEL_ASSESSMENT = COHERENT_BUT_BUDGETLESS_INDEPENDENT_POOLS
```

---

## 10. Connection ownership (architectural target)

| Question | Current proxy | Hybrid target |
|----------|---------------|---------------|
| Who owns client conn lifetime? | Tokio task / Hyper driver | One shard / event-loop owner |
| Who owns upstream? | Hyper client pool / lease | Same shard preferred |
| Local to one execution shard? | Weak (work stealing) | **YES** preferred |
| Cross-shard handoffs | Accept → tasks → client | Minimize to admit only |

NGINX lesson (architectural, not copy): process/event-loop **owns** connection; low work-stealing; fewer task abstractions → ~14 ctx/req (MC-B). Reproducible in Rust via mio/TPC **without** leaving Rust.

---

## 11. Shared state direction

Prefer (already partially present): immutable generations, Arc snapshots, worker-local caches. Avoid new central locks on proxy hot path. WAF freshness via generation handle, not global mutex per request.

---

## 12. Portability

| Platform | Strategy |
|----------|----------|
| Linux | Specialized dataplane (epoll/mio) allowed for proxy if portable fallback exists |
| macOS | Tokio/Hyper or kqueue mio fallback — iteration, not Linux evidence host |
| Windows | IOCP via portable runtime fallback — later |

A Linux-only dataplane **10%+** win is acceptable **only** with coherent portable control/fallback (DB1 / platform owners).

---

## 13. Dependency strategy

```text
RECOMMENDED =
  B/C hybrid: Hyper for control / optional H2;
  specialized H1 proxy dataplane;
  Cap067 unchanged;
  Tokio remains process substrate
```

Not ideology: MC-B proves Hyper+Tokio multi_thread proxy cannot reach NGINX class; ExyonQ integration alone (~15%) cannot close the gap.

---

## 14. Preserve / replace matrix

| Component | Current value | Execution | Perf health | Correctness | Migration value | ACTION |
|-----------|---------------|-----------|-------------|-------------|-----------------|--------|
| Cap067 / sendfile | High | OS epoll FSM | GOOD (P3) | Strong | Keep | **PRESERVE_AS_IS** |
| Small static | High | Cap067/wire | ACCEPTABLE | Strong | Incremental | **PRESERVE_WITH_ADAPTER** |
| Routing / generations | High | Snapshot | GOOD | Strong | Keep | **PRESERVE_AS_IS** |
| WAF | High | Wire+Hyper gates | N/A (security) | Strong | Adapter later | **PRESERVE_AS_IS** |
| Module API | High | Contracts | N/A | Strong | Keep | **PRESERVE_AS_IS** |
| Observability | Required | Tokio | N/A | — | Async offload | **PRESERVE_AS_IS** |
| Config reload | High | Tokio | GOOD | Strong | Keep | **PRESERVE_AS_IS** |
| Control HTTP API | Medium | Hyper+Tokio | GOOD | Strong | Keep | **PRESERVE_AS_IS** |
| Proxy Hyper+Tokio hot path | High product / poor perf | Hyper client+server | **POOR** | PXDP correctness OK | High if specialized | **REPLACE_EXECUTION_LAYER** |
| PXDP feature stack | Design value; perf fail | Wire+Hyper | PR-C fail | Confirmed | HOLD | **HOLD / ADAPT after prototype** |
| FastCGI | Product | spawn_blocking | UNKNOWN | Live | Independent | **PRESERVE_AS_IS** |
| P4 micro-opt campaign | Low residual | — | Exhausted | — | None | **RETIRE** |

---

## 15. CASE

```text
CASE = GA-B
CASE_NAME = HYBRID_EXECUTION_ARCHITECTURE_RECOMMENDED
```

| Alternative | Why not |
|-------------|---------|
| GA-A | PR-C + MC-B: specialization under current Hyper+Tokio stacking understates substrate mismatch |
| GA-C | Overclaim — Tokio process-wide redesign unnecessary (RT-C, Cap067, control) |
| GA-D | Overshoots — Cap067/control should not be replaced |
| GA-E | Rejected — no overwhelming evidence; destroys P3 wins |
| GA-F | Evidence sufficient |

---

## 16. Next prototype (DO NOT BUILD)

```text
NEXT_PROTOTYPE_PROPOSAL =
  ISOLATED_NON_PRODUCT_MIO_OR_THREAD_PER_CORE_EVENT_LOOP_H1_REVERSE_PROXY
  real TCP; keepalive GET relay; same Netcup P4 control; no ExyonQ product link

EXPECTED_VALUE_GATE =
  PRIMARY >= 15% median RPS vs ExyonQ PXDP on identical P4 harness
  SECONDARY material ctx/req and/or CPU µs/req reduction
  NOT REQUIRED to beat NGINX in v1

FORBIDDEN_AS_NEXT =
  Hyper+Tokio micro-opts; PXDP-P6; body relay; Cap067 removal; worker default change
```

---

## 17. Roadmap effect

```text
P4_MICRO_OPTIMIZATION_CAMPAIGN = STOPPED
REQUEST_BODY_RELAY = HOLD
PXDP_P6 = HOLD
PXDP_FEATURE_EXPANSION = HOLD
H2_H3_EXPANSION = HOLD_IF_EXECUTION_DEPENDENT
WEBSOCKET_CONNECT_GROWTH_ON_CURRENT_SHELL = HOLD
MULTI_CONCURRENCY_SWEEP = HOLD
WORKER_DEFAULT_CHANGE = NOT_AUTHORIZED
CAP067_REMOVAL = NOT_AUTHORIZED
```

---

## 18. External references (lessons only)

| System | Lesson |
|--------|--------|
| NGINX | Connection-owned event loop; low ctx; process/worker locality |
| OLS | Competitive proxy class on same host (PR-C) |
| HAProxy | Different optimization target (not throughput peer at this P4 config) |
| Pingora / Glommio / monoio / Seastar-like | Thread-per-core / ownership models in Rust/C++ — study only |

No code copy. No fabricated internet claims beyond prior ExyonQ evidence packs.

---

## Follow-on prototype (completed)

```text
WIP = V044_SPECIALIZED_PROXY_EVENT_LOOP_CONTROL
CASE = EP-C
SELECTED_MODEL = C_THREAD_PER_CORE_PLUS_MIO
PROTOTYPE_VS_EXYONQ_RPS = +38.18%
PROTOTYPE_VS_NGINX = +3.67%
PROTOTYPE_VS_OLS = -1.22%
SPECIALIZED_PROXY_DATAPLANE_JUSTIFIED = YES
```

## Follow-on integration design (completed)

```text
WIP = V044_SPECIALIZED_PROXY_DATAPLANE_CONTRACT_AND_INTEGRATION_DESIGN
CASE = SD-B
GEOMETRY = VALIDATED (do not merge prototype sources)
FIRST_SLICE = SDP-P1 (NOT AUTHORIZED)
DOC = docs/architecture/v044-specialized-proxy-dataplane-contract-integration-design.md
```

GA-B hybrid direction **confirmed by measurement and contracts**: preserve Cap067/Tokio/Hyper role-scope; next product step is owner-gated SDP-P1.
