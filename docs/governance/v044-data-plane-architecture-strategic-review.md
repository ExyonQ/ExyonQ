# V044 — Data plane architecture strategic review

```text
STATUS = COMPLETE
CASE = DP-C
WIP = V044_DATA_PLANE_ARCHITECTURE_STRATEGIC_REVIEW
MODE = READ_ONLY_CROSS_PROFILE_ARCHITECTURE_AND_PRODUCT_VALUE_REVIEW
PRODUCT_MUTATION = NO
ARCHITECTURE_MUTATION = NO
EXPERIMENTAL_CODE = NO
P5_STARTED = NO
P5_TO_P14_FROZEN = YES
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
PUBLIC_MIGRATION_CLAIMS = FORBIDDEN

SOURCE_HEAD = ade997a362e60d1dc6eafc984da454930600845a
SOURCE_TREE = 4ae6ffc7528d0881be23fcac246c345dcb9a1606
SOURCE_BINARY_SHA256 = e28c0abe18587f2d911d492be64cbec42372383e011417a6c578be52da5ba459
GOVERNANCE_HEAD_AT_REVIEW = current lab tree (read-only review)
```

## Executive verdict

```text
V044_DATA_PLANE_ARCHITECTURE_STRATEGIC_REVIEW_STATUS = COMPLETE
CASE = DP-C   # DEDICATED_PROXY_DATA_PLANE_JUSTIFIED (design-only; no implementation authorized)

ARCHITECTURE_MODEL = MODEL_C (MIXED)
  # Static/Cap067 large-payload path is structurally sound (P3 proof).
  # Small-static and proxy paths pay disproportionate fixed/integration tax.

ARE_WE_OPTIMIZING_LOCAL_MAXIMA = STRONGLY_SUPPORTED
  # Repeated profile-specific repair loops (especially P1 + P4) without resolving
  # execution-model fragmentation on the proxy path.

RECOMMENDED_ARCHITECTURE_DIRECTION =
  Preserve Cap067 static/sendfile data plane; design a dedicated proxy execution
  shell with fewer task/channel/body transitions — not continued P4 slice stacking.

SHOULD_V044_P4_PROXY_WIRE_BODY_PIPELINE_SLIM_PROCEED_NOW = NO

NEXT_DESIGN_WIP_PROPOSAL = HOLD — superseded strategically by GA-B hybrid review
  # completed: V044_PROXY_EXECUTION_DATA_PLANE_ARCHITECTURE_DESIGN — CASE PXDP-B
  # completed: V044_PROXY_EXECUTION_DATA_PLANE_CONTRACTS_IR — CASE P0-A
  # completed: V044_PXDP_PERFORMANCE_REALITY_CHECK — CASE PR-C (perf material FAIL)
  # completed: V044_GLOBAL_EXECUTION_ARCHITECTURE_STRATEGIC_REVIEW — CASE GA-B
NEXT_PRODUCT_WIP_PROPOSAL = NONE
NEXT_MEASUREMENT_WIP_PROPOSAL = isolated non-product mio/TPC proxy prototype (owner auth; not started)
P4_MICRO_OPTIMIZATION_CAMPAIGN = STOPPED
PXDP_P6 = HOLD
REQUEST_BODY_RELAY = HOLD

GLOBAL_MIGRATION_VALUE = NOT_PROVEN
MARKET_READINESS = NOT_READY
```

---

## 1. Why this review exists

P1–P4 evidence shows a repeating pattern: competitive weakness → causal investigation →
targeted repair → partial improvement → remaining gap → next root. P3 proves the **current
Rust product can be competitive** when kernel sendfile amortizes fixed cost. P4 proves the
**proxy path uses a different, materially heavier execution model** (~24% RPS deficit; +39%
CPU/req vs NGINX) that has already consumed multiple WIPs (baseline, profiling, Cap034
contract, Cap034 A/B +5.27%, execution-tax decomposition, proposed body-pipeline slice)
without approaching parity.

This is **not** authorization to rewrite ExyonQ, replace Hyper/Tokio, or continue
profile-by-profile optimization while the proxy architecture question remains open.

---

## 2. Primary questions (answers)

### What architectural factors cause repeated profile-specific optimization?

```text
WHAT_ARCHITECTURAL_FACTORS_CAUSE_EXYONQ_TO_REQUIRE_REPEATED_PROFILE_SPECIFIC_OPTIMIZATION =
  1. EXECUTION_MODEL_FRAGMENTATION — at least four coexisting models (see §5):
     Cap067 epoll FSM, Tokio async tasks, Hyper connection drivers, hybrid handoff seams.
  2. FAST_PATH_PROLIFERATION — static wire, site_static, sendfile FSM, proxy wire,
     Hyper fallback, WAF branches; each with distinct stage geometry.
  3. FIXED_COST_SENSITIVITY — ~11 µs class framework cost dominates P1/P4 1 KiB workloads;
     amortized only when payload work is large (P3).
  4. PROXY_SUBSYSTEM_INTEGRATION_TAX — ExyonQ→Hyper client geometry (not Hyper-intrinsic):
     mpsc dispatch, pool checkout, BoxBody/body.frame pipeline, ~133 ctx switches/req.
  5. MONOLITH_MEMORY_FOOTPRINT — shared fat binary + runtime state hurts P1/P4 RSS vs rivals.
  6. CORRECTNESS_REPAIR_OVERHEAD — performance work repeatedly surfaces distributed invariants
     (FD identity, Cap067 miss/hang, Host ranking, Cap034 framing) requiring audit/repair WIPs.
```

Evidence: **STRONGLY_SUPPORTED** (cross-profile review CASE B; P4 PX-B decomposition;
architecture fitness before P4).

### Can current architecture reach sustainable cross-profile competitiveness with bounded targeted evolution?

```text
CAN_CURRENT_ARCHITECTURE_REACH_SUSTAINABLE_CROSS_PROFILE_COMPETITIVENESS_WITH_BOUNDED_TARGETED_EVOLUTION =
  PARTIAL / NOT_PROVEN

  PROVEN: large static (P3) at migration bar for COMPETITIVE_WIN=YES.
  NOT_PROVEN: P1 RPS vs NGINX (−8.8% post-G1 despite CPU win); P4 proxy (−24% RPS).
  BOUNDED: stacked P4 local slices (body + scheduler + Cap034) ≈12–22% non-overlapping
  recoverable RPS — insufficient alone to close ~24% gap; high WIP repeat risk for P5–P14.
```

### Does the data plane need a more fundamental redesign?

```text
DOES_THE_DATA_PLANE_NEED_A_MORE_FUNDAMENTAL_REDESIGN =
  PROXY_SUBSYSTEM: YES (bounded redesign — dedicated execution shell)
  STATIC_SUBSYSTEM: NO (evolve incrementally; Cap067 path is competitive at P3)
  FULL_PRODUCT: NO (control plane, config, modules, P3 path preserved)
```

---

## 3. Authoritative P1–P4 competitive truth (unchanged)

| Profile | ExyonQ RPS | vs NGINX | CPU/req vs NGINX | RSS ExyonQ | P50/P95/P99 (ExyonQ) | COMPETITIVE_WIN |
|---------|----------:|---------:|-----------------:|-----------:|----------------------|-----------------|
| P1 1 KiB | 154062 | −8.8% | **−19.3%** (win) | 13.86 MiB | 0.79 / 1.29 / 1.71 ms | NOT_PROVEN |
| P2 64 KiB | 104041 | **+15.3%** | **−16.0%** (win) | 21.64 MiB | 1.20 / 1.87 / 2.41 ms | NOT_PROVEN |
| P3 1 MiB | 8683 | +1.0% | **−0.4%** (win) | 71.95 MiB | 13.95 / 19.70 / 25.37 ms | **YES** |
| P4 proxy 1 KiB | 73097 | −24.2% | **+39.4%** (loss) | 33.18 MiB | 1.66 / 2.71 / 3.53 ms | NOT_PROVEN |

Canonical P4 references: NGINX 96447 RPS; OLS 97310 RPS. Profiling-session numbers
(e.g. ~74.1k RPS in PX-B) are **not** canonical competitive authority.

---

## 4. Architecture model classification

```text
MODEL_A COMMON_EFFICIENT_DATA_PLANE + small differences = REJECTED
  # P4 proxy path is not a small delta from static path.

MODEL_B GENERAL_FRAMEWORK_OVERHEAD + multiple execution models + profile repairs = BOUNDED
  # Accurate for P1 small-static residual + P4 proxy; overstated for P3.

MODEL_C MIXED — some subsystems sound, others need redesign = STRONGLY_SUPPORTED
  # Cap067/sendfile static: sound (P3).
  # Tokio/Hyper proxy wire: over-layered (P4 PX-B).
```

---

## 5. Data-plane inventory (summary)

| Subsystem | Execution | Ownership | Thread/task | Buffer | Hot-path status |
|-----------|-----------|-----------|-------------|--------|-----------------|
| Accept | Tokio + epoll pool | stream → task | handoff | kernel socket | ESSENTIAL |
| Static Cap067 | epoll FSM | session-owned FD | epoll thread | sendfile | **COMPETITIVE (P3)** |
| Static first req | Tokio → Cap067 divert | seam | 2 models | Bytes head | P1 sensitivity |
| HTTP parse (proxy) | Tokio | connection task | async poll | Bytes | ESSENTIAL |
| Routing / generation | Tokio | Arc snapshot | read-mostly | none | ESSENTIAL |
| WAF | Tokio | hook | early-out bench | — | SECURITY (paid when on) |
| Proxy wire | Tokio + Hyper | cluster RwLock | 4–6 tasks/req | BoxBody/Frames | **P4 BOTTLENECK** |
| Hyper client pool | Hyper driver task | pool checkout | mpsc | Incoming | **P4 BOTTLENECK** |
| Response writer | Tokio | progressive TE | frame loop | Vec/chunk | **P4 BOTTLENECK** |
| Observability | Tokio | hooks | entered even if no-op | — | OPTIONAL_BUT_PAID |

```text
EXECUTION_MODEL_COUNT = 4 (minimum)
  1. Cap067 epoll sendfile FSM
  2. Tokio connection/task runtime
  3. Hyper client/server connection drivers
  4. epoll↔Tokio hybrid handoff seam (static + /api/ proxy entry)

DATA_PLANE_HANDOFF_COUNT_BY_PROFILE (bounded):
  P1/P2 static (steady): BOUNDED 0–1 after first request (Cap067 pump)
  P1 first request: BOUNDED 1 (Tokio→Cap067)
  P4 proxy: BOUNDED 4–6 task + 2–4 channel + 1 epoll→Tokio
```

---

## 6. Cross-profile path comparison

| Stage | P1/P2 static | P3 static | P4 proxy |
|-------|--------------|-----------|------------|
| Accept | Tokio/epoll | same | Tokio/epoll |
| Parse/route | wire/static | same | wire_dispatch |
| Security | WAF gate (off bench) | same | WAF gate (off bench) |
| Work | Cap067 sendfile | Cap067 sendfile (dominant) | Hyper upstream GET |
| Response | kernel TX | kernel TX | chunked TE userspace |
| Special fast paths | site_static, FD cache | same | proxy wire only |
| Cap067 | **YES** | **YES** | **NO** |
| Hyper client | NO (steady) | NO | **YES** |
| BoxBody/frame adapter | NO | NO | **YES** |

```text
COMMON_STAGES = accept, parse, route, generation pin, bench RAW policy off
PROFILE_SPECIFIC_BYPASSES = Cap067 (P1–P3), proxy wire (P4)
PROFILE_SPECIFIC_FAST_PATHS = site_static, sendfile FSM, might_use_proxy_wire
```

---

## 7. Why P3 works (mandatory)

```text
P3_SUCCESS_ROOTS =
  SENDFILE_AMORTIZES_FIXED_COST — STRONGLY_SUPPORTED
  KERNEL_DATA_MOVEMENT_DOMINATES — STRONGLY_SUPPORTED (cgroup system µs)
  LOWER_USERSPACE_STAGE_PERCENT — STRONGLY_SUPPORTED (~11 µs F vs ~100+ µs TX)
  FEWER_TASK_HANDOFFS_PER_BYTE — STRONGLY_SUPPORTED (Cap067 FSM vs proxy task stack)
  FEWER_ALLOCATIONS_PER_BYTE — BOUNDED (sendfile vs frame loop)

CAN_THE_SAME_ARCHITECTURAL_PRINCIPLES_BE_APPLIED_TO_SMALL_STATIC_AND_PROXY_PATHS =
  PARTIAL
  # Principle "minimize userspace work per byte" applies.
  # Mechanism "Cap067 sendfile" does NOT apply to reverse proxy streaming.
  # Proxy needs a different minimal shell: direct socket streaming with fewer async layers.
```

---

## 8. Why P1 still needs work

Documented P1 performance WIPs (governance index): **≥17** distinct investigations/repairs
including Root1/2/3, FD reuse, Cap067 miss/hang (H1), sessiontable, parallelism AB (AB3),
G1 worker geometry (product), deficit attribution, residual reprofiles, CPU efficiency causal,
OLS FD geometry, static TX convergence, final rebaseline.

```text
P1_OPTIMIZATION_WIP_COUNT = 17 (documented governance artifacts)
WHAT_EACH_FIXED = parallelism default (+10% class), TLS pin, read_state, FD reuse, hang class, etc.
NET_GAIN = CPU win vs NGINX; RPS still −8.8%; latency still behind NGINX
COMPLEXITY_ADDED = MEDIUM–HIGH (Cap067 seams, geometry coordination, multiple hot paths)
DEFECTS_INTRODUCED = FD reuse/hang class (H1), sessiontable investigations — repair WIPs required
AUDIT_REPAIR_WIPS_REQUIRED = ≥2 (H1, Cap034-adjacent Host ranking on wire)
ENGINEERING_COST_PER_GAIN = HIGH for residual −8.8% RPS (not closed)
```

```text
WHY_P1_REMAINS_MIXED =
  Wins CPU vs NGINX but loses RPS/latency — underutilization + fixed stage depth on 1 KiB.
  Not explained by higher CPU/req; parallelism root closed (G1); residual is architectural
  small-payload delivery + monolith footprint, not one more local knob.
```

---

## 9. Why P4 needs multiple roots

P4 WIPs to date: competitive baseline (P4-C2), path causal profiling (P4P-B), Cap034 contract
(C34-C), Cap034 A/B (AB-C34-B), execution tax (PX-B), body-pipeline slim (**HOLD**).

```text
P4_OPTIMIZATION_WIP_COUNT = 6 (measurement + experiment + proposed slice)
CLASSIFICATION = BOTH
  NORMAL_PRODUCT_HARDENING — baseline, profiling, contract review (necessary)
  ARCHITECTURAL_COMPLEXITY_SIGNAL — each slice (framing, body, scheduler) addresses symptoms
    of one integration stack; Cap034 +5.27% still leaves ~19% gap to NGINX

WHY_P4_IS_STRUCTURALLY_EXPENSIVE =
  Dual HTTP stacks (client + upstream), Tokio-only path (no Cap067), ~133 ctx switches/req,
  ~28.9M instructions/req, user CPU ~3.2× NGINX on P4 meter, coupled roots 1A+1F+1G.
```

---

## 10. Engineering economics ledger (proxy)

| WIP class | Count (P1–P4) | Notes |
|-----------|---------------|-------|
| P1 performance/investigation | 17 | Many closed; RPS gap remains |
| P2 measurement | 1 | Baseline only |
| P3 measurement | 1 | COMPETITIVE_WIN=YES |
| P4 measurement + experiment | 5–6 | Includes Cap034 AB, PX-B |
| Product repair (cross-profile) | ≥3 | G1, Root1/3, FD reuse |
| Audit/repair from perf work | ≥2 | H1 hang, wire Host ranking |

```text
TOTAL_PERFORMANCE_WIPS_P1_P4 = 25 (documented governance artifacts; measurement + repair + experiment)
TOTAL_PRODUCT_REPAIR_WIPS = BOUNDED 5–7 (product mutations with measured intent)
TOTAL_AUDIT_REPAIR_WIPS = BOUNDED 2–4 (correctness exposed by optimization)
TOTAL_EXPERIMENT_WIPS = 2 (Cap034 AB; body-pipeline slim HOLD/not completed)

ESTIMATED_COMPLEXITY_ACCUMULATION = HIGH
  # Multiple execution models + fast paths + experiment toggles; slice fixes do not simplify graph.
```

---

## 11. Anti-local-optimum

```text
ARE_WE_OPTIMIZING_EACH_PROFILE_TOWARD_A_LOCAL_MAXIMUM_WHILE_PRESERVING_A_SUBOPTIMAL_GLOBAL_ARCHITECTURE =
  STRONGLY_SUPPORTED

Evidence:
  - P4 Cap034 +5.27% does not change execution model; gap remains material.
  - Body-pipeline proposal targets 6–12% within same Tokio/Hyper geometry.
  - P1 residual pursued via stage/parallelism/FD WIPs without unified delivery model.
  - P3 win not leveraged to simplify proxy (different path entirely).
```

---

## 12. Common vs profile-specific improvements

| Improvement | Class | Cross-profile reach |
|-------------|-------|---------------------|
| G1 worker geometry (8/8/8) | COMMON | P1/P2/P3 (+P4 defaults) |
| Root1 TLS static pin | PROFILE (static) | P1–P3 |
| Root3 read_state | PROFILE (Cap067) | P1–P3 |
| FD reuse / H1 hang | PROFILE (Cap067) | P1–P3 |
| Cap034 framing AB | PROFILE (P4 wire) | P4 only |
| Body pipeline slim (proposed) | PROFILE (P4 wire) | P4 only |
| Deficit attribution / AB3 | MEASUREMENT | P1 |

```text
COMMON_DATA_PLANE_IMPROVEMENTS = G1 geometry, shared accept/epoll pool, generation snapshot model
PROFILE_SPECIFIC_IMPROVEMENTS = Cap067 sendfile stack, proxy wire/Hyper integration, Cap034
WORKAROUNDS = first-request Tokio→Cap067 divert (P1 seam)
CORRECTNESS_REPAIRS = H1, Host ranking, harness corrections (P4 invalid run)
```

---

## 13. Architecture principles evaluation

| Principle | Static P1–P3 | P4 proxy | Assessment |
|-----------|--------------|----------|------------|
| ONE_CONNECTION_OWNERSHIP_MODEL | BOUNDED (seam on first req) | FAIL (tasks + Hyper pool) | Proxy violates |
| MINIMAL_EXECUTION_HANDOFFS | GOOD post-Cap067 | FAIL (~133 ctx/req) | Proxy violates |
| WORKER_LOCAL_STATE | BOUNDED | NOT_MEASURED | Research |
| MINIMAL_ALLOCATIONS_PER_REQUEST | GOOD at P3 scale | FAIL (Bytes/malloc) | Proxy violates |
| MINIMAL_BUFFER_TRANSITIONS | GOOD (sendfile) | FAIL (frame/box) | Proxy violates |
| MINIMAL_DYN_DISPATCH_HOT_PATH | BOUNDED | FAIL (BoxBody/stream) | Proxy violates |
| SINGLE_ROUTING_DECISION_FLOW | PASS | PASS | OK |
| COMMON_ERROR_MODEL | BOUNDED | BOUNDED | OK |
| COMMON_BACKPRESSURE_MODEL | PASS (sendfile) | BOUNDED (frame loop) | OK if streaming preserved |
| COMMON_HTTP_BODY_MODEL | FAIL across profiles | FAIL | Fragmented |
| CONTROL_PLANE_OUT_OF_HOT_PATH | BOUNDED (generation pin) | BOUNDED | Acceptable |
| OBSERVABILITY_LOW_HOT_PATH_COST | BOUNDED (bench off) | BOUNDED | Paid hooks remain |
| SECURITY_GATES_WITHOUT_DUPLICATED_PIPELINE | BOUNDED | BOUNDED | WAF single gate |

---

## 14. Hot-path stage count

```text
CURRENT_COMMON_HOT_PATH_STAGE_COUNT =
  P3 minimal: BOUNDED 6–8 (accept → route → Cap067 sendfile → complete)
  P4 proxy: BOUNDED 18–22 (documented PX-B graph)

MINIMAL_PLAUSIBLE_STAGE_COUNT =
  BOUNDED 5–7 for any profile (accept → parse → route/policy → operation → write → lifecycle)

CURRENT_TO_MINIMAL_STAGE_DELTA =
  P3: NEAR (≈0–2 extra)
  P4: LARGE (≈10–15 extra userspace stages)
```

---

## 15. Options evaluation

### A — Continue targeted evolution

```text
OPTION_A_CONTINUE_TARGETED_EVOLUTION =
  EXPECTED_ENGINEERING_COST = VERY_HIGH (P5–P14 repeat risk)
  EXPECTED_ARCHITECTURE_COMPLEXITY = HIGH (accumulating slices)
  EXPECTED_P4_FINAL_RPS_RANGE = BOUNDED 82k–89k RPS (73k × 1.12–1.22 stacked; not measured)
  EXPECTED_P5_P14_REPEAT_RISK = HIGH
  LIKELIHOOD_OF_PROFILE_BY_PROFILE_REPEAT = STRONGLY_SUPPORTED
  SUSTAINABILITY = LOW–MEDIUM
  RECOMMENDATION = REJECT as primary strategy; acceptable only for bounded correctness fixes
```

### B — Common data-plane refactor

```text
OPTION_B_COMMON_DATA_PLANE_REFACTOR =
  SCOPE = Shared accept/route/response shell; unify connection ownership at boundary
  EXPECTED_AFFECTED_PROFILES = P1, P4 (P2/P3 partial)
  EXPECTED_GAIN_RANGE = P1 BOUNDED 3–8% RPS; P4 BOUNDED 5–10% (hypothesis; not measured)
  RISK = MEDIUM–HIGH
  COMPLEXITY = HIGH
  CAN_BE_INCREMENTAL = YES (phased shell around existing operations)
  RECOMMENDATION = CONDITIONAL — secondary to proxy-specific design; helps seams, not full P4 gap
```

### C — Dedicated proxy data plane

```text
OPTION_C_DEDICATED_PROXY_DATA_PLANE =
  EXPECTED_GAIN_RANGE = BOUNDED 15–25% P4 RPS (decomposition option F; PLAUSIBLE not proven)
  COMPLEXITY = HIGH
  RISK = HIGH (correctness, WAF, reload, streaming)
  DUPLICATION_RISK = MEDIUM (must not fork HTTP semantics)
  MAINTENANCE_COST = MEDIUM if bounded to proxy module
  P4_SPECIFIC_OR_GENERAL_PROXY_VALUE = GENERAL for P4, P8+, future proxy-class profiles
  COEXISTENCE = WAF, routing, generations, observability — feasible with explicit contract
  RECOMMENDATION = **PRIMARY** — design WIP authorized; implementation NOT authorized here
```

### D — Major data-plane redesign (not full rewrite)

```text
OPTION_D_MAJOR_DATA_PLANE_REDESIGN =
  WHY = Only if proxy shell (C) insufficient AND P1 residual requires same ownership model
  EXPECTED_PROFILES_IMPROVED = P4 primary; P1 secondary
  EXPECTED_GAIN_RANGE = NOT_BOUNDED
  MIGRATION_COMPLEXITY = VERY_HIGH
  CORRECTNESS_RISK = HIGH
  TIME_TO_REACH_PARITY = NOT_ESTIMABLE
  RECOMMENDATION = DEFER — try bounded C first; preserve P3 Cap067 investment
```

### E — Full rewrite

```text
OPTION_E_FULL_REWRITE =
  FULL_REWRITE_JUSTIFIED = NO
  # P3 competitive win; bounded roots; incremental path exists for proxy shell.
```

---

## 16. Language and dependency decisions

```text
RUST_LANGUAGE_CHOICE = KEEP
  # P3 demonstrates competitive Rust on Cap067 path; P4 failure is integration geometry.

HYPER_DEPENDENCY = TARGETED_REDUCTION
  # Not REPLACE — PX-B rejects Hyper-intrinsic blame; integration/body path is the lever.

TOKIO_RUNTIME = TARGETED_REDUCTION
  # Not REPLACE — scheduler cost is ExyonQ task geometry (133 ctx/req), not Tokio-intrinsic.
```

---

## 17. Fast-path proliferation

```text
FAST_PATH_COUNT = BOUNDED 6–8
  site_static, static wire, Cap067 sendfile FSM, epoll keepalive pump, proxy wire,
  Hyper full-stack fallback, WAF wire branch, bench dispatch shortcuts

FAST_PATH_SEMANTIC_DUPLICATION = BOUNDED (response framing, connection lifecycle, routing entry)

FAST_PATH_MAINTENANCE_RISK = MEDIUM–HIGH

ARE_FAST_PATHS_CONVERGING_OR_FRAGMENTING_THE_ARCHITECTURE = FRAGMENTING
  # Static and proxy paths diverge early; repairs do not converge them.
```

---

## 18. Correctness cost

```text
PERFORMANCE_REPAIR_DEFECT_RATE_PROXY = BOUNDED MEDIUM
  # FD/TOCTOU, Cap067 miss/hang, Host ranking, harness invalid P4 run, Cap034 semantics.

ARCHITECTURAL_INVARIANT_FRAGMENTATION = STRONGLY_SUPPORTED
  # Invariants distributed across wire_dispatch, wire_conn, epoll_worker, hyper_forward,
  # Cap034 contract — optimization repeatedly surfaces cross-module coupling.
```

---

## 19. Security / observability tax

```text
WAF_OFF_COST = BELOW_RESOLUTION on P1 AB3; still entered on P4 path
OBSERVABILITY_OFF_COST = BOUNDED (hooks entered; bench no-op)
GENERATION_CHECK_COST = BOUNDED (pin/snapshot read on hot path)
CAN_THEIR_DATA_PLANE_INTEGRATION_BE_STRUCTURALLY_CHEAPER = YES (design question; not removal)
```

---

## 20. Memory architecture

```text
P1/P4_MEMORY_DISADVANTAGE_PRIMARY = fat monolith Private_Clean (~9–10 MiB) + runtime anon
P4_INCREMENTAL = Hyper pool + proxy cluster state (~8–12 MiB class; NOT_MEASURED breakdown)
SHARED_VS_PROFILE = SHARED base + P4 proxy incremental
CAN_CONTROL_AND_DATA_PLANE_RESIDENCY_BE_SEPARATED = BOUNDED YES (process/OCI split — not authorized)
```

---

## 21. Performance ceiling scenarios (bounded; not measured)

```text
S1_LOCAL_OPTIMIZATION_EXPECTED_P4_RANGE =
  RPS: BOUNDED 82k–89k (73k canonical × 1.12–1.22 stacked local slices)
  Still BEHIND NGINX ~96k; tails NEUTRAL risk
  S1_COMPLEXITY_COST = HIGH (continued WIP stack)

S2_COMMON_DATA_PLANE_EXPECTED_CROSS_PROFILE_EFFECT =
  P1: BOUNDED 3–8% RPS; P2: LOW; P3: NEUTRAL; P4: BOUNDED 5–10%
  Evidence: HYPOTHESIS — shared ownership shell

S3_PROXY_DATA_PLANE_EXPECTED_P4_EFFECT =
  RPS: BOUNDED 15–25% (PLAUSIBLE from PX-B option F)
  STATIC_PROFILE_EFFECT = NEUTRAL if Cap067 preserved
```

---

## 22. P5–P14 repeat-risk (no fabricated numbers)

| Profile class | Dominant mechanism today | Repeat-risk |
|---------------|-------------------------|-------------|
| P5–P7 (TLS/HTTP2/H2 proxy) | Hyper + Tokio stack | **HIGH** |
| P8 SSE/streaming | progressive TE + Tokio | **HIGH** |
| P9–P10 WebSocket/WASM | separate fast paths | **MEDIUM–HIGH** |
| P11+ TLS termination | rustls + Hyper | **HIGH** |
| Static large (P3-like) | Cap067 | **LOW** |

```text
P5_TO_P14_REPEAT_RISK = HIGH without proxy/static execution unification strategy
```

---

## 23. Migration value

```text
WHY_SHOULD_AN_ADMIN_MIGRATE = NOT_PROVEN at global bar
  P3: PARTIAL (large static competitive)
  P4: NOT_PROVEN (material RPS+CPU loss)
  P1: NOT_PROVEN (RPS/latency vs NGINX)

EXPECTED_MIGRATION_VALUE_DELTA_OPTION_A = NONE to NEGATIVE (complexity without closure)
EXPECTED_MIGRATION_VALUE_DELTA_OPTION_B = PARTIAL (ops simplicity if shell succeeds)
EXPECTED_MIGRATION_VALUE_DELTA_OPTION_C = MATERIAL for proxy-class workloads (if gains realized)
EXPECTED_MIGRATION_VALUE_DELTA_OPTION_D = UNKNOWN
```

---

## 24. Decision matrix

| Option | Perf upside | Cross-profile | Eng. complexity | Defect risk | Maintenance | Time to validate | Rollback | Cleanliness | Migration value |
|--------|-------------|---------------|-----------------|-------------|-------------|------------------|----------|-------------|-----------------|
| A Continue slices | BOUNDED | LOW | VERY HIGH | MEDIUM | HIGH | SHORT per slice | EASY | LOW | LOW |
| B Common refactor | MEDIUM | MEDIUM | HIGH | MEDIUM | MEDIUM | LONG | MEDIUM | MEDIUM | PARTIAL |
| C Dedicated proxy DP | HIGH (P4) | MEDIUM | HIGH | HIGH | MEDIUM | LONG | HARD | HIGH | MATERIAL |
| D Major redesign | UNKNOWN | HIGH | VERY HIGH | VERY HIGH | HIGH | VERY LONG | HARD | MEDIUM | UNKNOWN |
| E Full rewrite | UNKNOWN | ALL | EXTREME | EXTREME | EXTREME | EXTREME | IMPOSSIBLE | UNKNOWN | NOT JUSTIFIED |

---

## 25. Body-pipeline experiment hold

```text
V044_P4_PROXY_WIRE_BODY_PIPELINE_SLIM = HOLD (owner strategic review)
SHOULD_V044_P4_PROXY_WIRE_BODY_PIPELINE_SLIM_PROCEED_NOW = DEFER_PENDING_ARCHITECTURE_CHANGE

WHY:
  - Isolated slice within same Tokio/Hyper execution model.
  - Does not address ROOT_1F (scheduler/task geometry) or ROOT_1A_2 (mpsc) — coupled to gap.
  - Risk of another +5–12% local win that still leaves material deficit and encourages
    continued profile surgery.
  - May remain valuable as DIAGNOSTIC ONLY if owner later authorizes bounded A/B to
    validate ROOT_1A_3+1G_2 slice — not as productization path without architecture decision.
```

See: [v044-p4-proxy-wire-body-pipeline-slim.md](v044-p4-proxy-wire-body-pipeline-slim.md)

---

## 26. ADR-021 design review (pre-code — N/A)

No product mutation in this WIP. Design pattern for recommended direction:

```text
PATTERN_SELECTED = Dedicated worker-local proxy connection state + minimal streaming transfer
NO_NEW_PATTERN_ALTERNATIVE = ACCEPTED (reuse existing routing/WAF contracts; new execution shell only)
```

---

## 27. Related documents

- [v044-cross-profile-performance-root-cause-review-p1-p4.md](v044-cross-profile-performance-root-cause-review-p1-p4.md)
- [v044-p4-proxy-execution-tax-decomposition.md](v044-p4-proxy-execution-tax-decomposition.md)
- [v044-p4-proxy-path-causal-profiling.md](v044-p4-proxy-path-causal-profiling.md)
- [v044-architecture-fitness-review-before-p4.md](v044-architecture-fitness-review-before-p4.md)
- [v044-cap034-framing-keepalive-bounded-value-ab.md](v044-cap034-framing-keepalive-bounded-value-ab.md)
- [performance-competitive-admission.md](performance-competitive-admission.md)

```text
NOTION_UPDATE_REQUIRED = YES (owner reconciliation — not verified by agent)
DOCS_RECONCILED = YES
PUSH = NO
```
