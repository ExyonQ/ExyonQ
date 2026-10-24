# V044 Core architecture reconciliation — Competitive Frontier

```text
WIP = V044_CORE_ARCHITECTURE_RECONCILIATION_FOR_COMPETITIVE_FRONTIER
PHASE = CORE_REVIEW_PHASE_0
MODE = READ_ONLY_CORE_ARCHITECTURE_REVIEW
CASE = CFR-B
PRODUCT_MUTATION = NO
ARCHITECTURE_MUTATION = NO
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
EVIDENCE = .exyonq-local/evidence/core-architecture-reconciliation/20260828-122859/
PHASE_1_AUTHORIZED = NO
```

Governance: [`../governance/v044-core-architecture-reconciliation-competitive-frontier.md`](../governance/v044-core-architecture-reconciliation-competitive-frontier.md)  
Performance budget: [`../performance/v044-core-architecture-performance-budget-contract.md`](../performance/v044-core-architecture-performance-budget-contract.md)

Lineage: GA-B → EP-C → SD-B → SDP-P1/SP1-C → IL-F → SPPG-C → **Phase 0 CFR** (this doc)

---

## 1. Primary question

```text
HOW_MUST_CURRENT_EXYONQ_BE_RECONCILED_WITH_THE_NEW_COMPETITIVE_FRONTIER
ARCHITECTURE_WITHOUT_UNNECESSARILY_REWRITING_ALREADY_VALID_PRODUCT_LOGIC =
```

**Answer (Phase 0):** Preserve product semantics (routing IR, WAF policy, generations, Cap067 static, FastCGI, Module API, observability contracts, error/commit rules). Replace **only** the competitive proxy **execution geometry** that IL-F/SPPG show cannot deliver the Competitive Frontier while co-resident with Cap067/Tokio. Do not whole-product rewrite. Do not revive extreme +25/−10.

---

## 2. Strategic authority

| Item | Status |
|------|--------|
| Extreme +25/−10 | **CLOSED** (SPPG-C kill switch) |
| Competitive Frontier target | ≈ SPPG vs OLS: RPS +9.3%, CPU +22.9% better, MEM +17.3%, P95 +16.5%, P99 +14.1% |
| SPPG | **ARCHITECTURE_TARGET_REFERENCE** — not shipping baseline |
| SDP-P1 productization | **CLOSED** (IL-F) — hold gated OFF |
| PXDP | **HOLD** |
| Cap067 | **PRESERVE** static execution |

SPPG correctness was PARTIAL; security NOT_EXECUTED — shipping admission still requires full product gates.

---

## 3. Current architecture (source SoT)

Single process `exyonq serve`: one production Tokio multi-thread runtime (`cli/exyonq`); Cap067 OS epoll pool in-process; default Tokio accept + `SO_REUSEPORT`; Hyper for many H1/H2 paths; proxy upstream via **global** Hyper client pools; generation via `SharedServerState` (`RwLock<Arc<ServerState>>`) + `AtomicU64`; routing SSOT `RuntimePlan`/`RouteIndex` (linear Cap033); WAF generation-frozen with wire inspect gate (ARCH-002).

Full map: evidence `architecture-map.md`, `execution-topology.md`.

```text
DEFAULT_PARALLELISM_COORDINATION_EXISTS = YES (runtime_parallelism.rs)
GLOBAL_PARALLELISM_SSOT_EXISTS = PARTIAL (defaults coordinated; env overrides independent)
GLOBAL_SUM_BUDGET_SSOT = NO
OVERSUBSCRIPTION_RISK = MED
LARGE_CPU_SERVER_RISK = HIGH (MAX_WORKERS=16; independent pools)
```

---

## 4. ADR-021 process model

**Alphabet warning:** SPPG ADR used **A**=same-process / **B**=separate-process. This Phase-0 table uses different letters. **Freeze by named enum only** — never “option A/B” alone.

| CFR ID | Named model | Verdict for shipping |
|--------|-------------|----------------------|
| CFR-PM-SEPARATE | `TRUE_SPLIT_PLANE_SEPARATE_PROCESS` | **Preferred isolation for competitive H1 proxy DP** (only geometry with measured soft-frontier deltas) |
| CFR-PM-SAME-STRICT | `SAME_PROCESS_STRICT_EXECUTION_DOMAINS` | **Closed as SDP-style path** (IL-F); reopen only with new isolation proof |
| CFR-PM-CURRENT-TARGETED | `CURRENT_PROCESS_TARGETED_EXECUTION_REPLACE` | **No frontier evidence**; IL-F contamination risk if specialized engine co-resides; **not** proven impossible for soft frontier; **rejected as sole path pending owner** |
| CFR-PM-HOLD | `NO_NEW_PATTERN` | Hold for Cap067/control; not frontier path |
| CFR-PM-APPLIANCE | Appliance / SHM | Deferred |

### Recommended pending owner freeze

```text
TARGET_PROCESS_MODEL = HYBRID
  CONTROL + Cap067_STATIC + FastCGI_bridge + H2/H3/control_API = CURRENT_PROCESS
  COMPETITIVE_H1_PROXY_DATAPLANE = HARD_ISOLATED
  PROXY_DP_ISOLATION_PREFERRED = SEPARATE_PROCESS   # named; = SPPG selected B, NOT SPPG letter A
PER_REQUEST_CONTROL_PLANE_INVOLVEMENT_TARGET = 0
TOKIO_IN_COMPETITIVE_PROXY_REQUEST_PATH_TARGET = NO
CAP067_IN_COMPETITIVE_PROXY_DP = NO
HYPER_FALLBACK_ON_COMPETITIVE_H1 = FORBIDDEN_UNLESS_EXPLICIT_ADMISSION_FEATURE
COMPETITIVE_H1_ACCEPT_OWNER = OWNER_FREEZE_REQUIRED
LISTENER_EXCLUSIVITY = OWNER_FREEZE_REQUIRED
SPLIT_PLANE_BOUNDARY = RECOMMENDED_PENDING_OWNER_FREEZE
```

Tech-lead draft preferred **CFR-PM-CURRENT-TARGETED** after extreme STOP; owner Competitive Frontier contract cites SPPG as architecture target → Phase 0 elevates **hard isolation (SEPARATE_PROCESS preferred)** for the proxy dataplane while preserving Cap067/Tokio for control/static. **Owner must freeze before Phase 1.**

---

## 5. Classification summary

| Class | Count (approx) | Examples |
|-------|----------------|----------|
| PRESERVE | ~12 | Cap067 static FSM, FastCGI, error/commit, hop-by-hop contract, BackendTable slots |
| PRESERVE_WITH_ADAPTER | ~10 | Module API, Hyper role-scoped, observability emit, Cap067 budget |
| MOVE_TO_CONTROL_PLANE | ~4 | config parse, plan compile, control socket, export sinks |
| COMPILE_TO_DATAPLANE | ~8 | RouteIndex, WAF rules, timeouts, rate local, generation projection |
| REPLACE_EXECUTION_ONLY | ~5 | Hyper upstream client/pools, Cap034 wire KA model, Tokio proxy scheduling |
| RETIRE (later) | 2 | SDP-P1 product path, PXDP design as execution plan |
| UNKNOWN_NEEDS_PROOF | ~6 | same-process isolation proof, module body tax, NUMA numbers, H2 role |

Exhaustive table: evidence `preserve-matrix.md`.

---

## 6. Subsystem classifications (headline)

| Area | Classification |
|------|----------------|
| Routing | COMPILE_TO_DATAPLANE |
| WAF | COMPILE_TO_DATAPLANE (no weaken) |
| Generation/reload | COMPILE_TO_DATAPLANE + control publish |
| Cap067 static | PRESERVE; THREAD_GEOMETRY PRESERVE_WITH_ADAPTER; PROCESS_RELATION PRESERVE for static |
| FastCGI | PRESERVE (initial migration NO) |
| Module API | PRESERVE_WITH_ADAPTER |
| Observability | PRESERVE_WITH_ADAPTER (hot bookkeeping) + MOVE export to control |
| Body/backpressure | PRESERVE contract; REPLACE_EXECUTION_ONLY impl |
| Timers | REPLACE_EXECUTION_ONLY on proxy DP (shard-local) |
| Error model | PRESERVE |
| Tokio | PRESERVE substrate; not competitive proxy request path |
| Hyper | ROLE_SCOPED PRESERVE_WITH_ADAPTER; REPLACE proxy hot execution |
| SDP-P1 | HOLD now / RETIRE later — do not productize |
| PXDP | HOLD / RETIRE later as execution program |
| Upstream pools | REPLACE_EXECUTION_ONLY → shard-local |

---

## 7. Dataplane boundary

| Responsibility | Separation class |
|----------------|------------------|
| Routing / WAF / header / timeout / gen id | EXECUTE_FROM_IMMUTABLE_COMPILED_STATE |
| Upstream select / local rate / KA pools | REQUIRES_LOCAL_MUTABLE_SHARD_STATE |
| Config parse / reload orchestration / exporters / admin | REQUIRES_CONTROL_PLANE |
| Mutating module body transform | CANNOT_CURRENTLY_SEPARATE without admission design |
| Distributed rate cluster | UNKNOWN / out of initial scope |

---

## 8. Connection ownership target

```text
CLIENT_OWNER_ONE_SHARD = FEASIBLE (SPPG/SDP geometry; productization CONDITIONAL)
UPSTREAM_POOL_SHARD_LOCAL = FEASIBLE (target); current = GLOBAL Hyper OnceLock
GLOBAL_POOL_REMOVAL_SEMANTIC_COST = MED (reuse accounting change)
```

---

## 9. Parallelism / NUMA (read-only)

```text
EXYONQ_PARALLELISM_BUDGET_MODEL =
  single detected_parallelism SoT
  + explicit sum budget across Tokio + Cap067 + DP shards + FCGI blocking
  + no hardcoded workers=4
  + NUMA: shard/CPU binding design for 8/16/32/64/128 — not implemented in Phase 0
```

Portability: Linux epoll primary; macOS kqueue conceptually via mio-class DP; Windows IOCP future — no Windows work now.

---

## 10. Migration sequence

Phases 1–9 frozen in evidence `migration-sequence.md` and governance. **Phase 1 not authorized.**

Phase 1 scope remains: dataplane foundations only (isolation, shard ownership, immutable gen publish, budget, feature gate OFF, zero RPC/request).

---

## 11. Zero-fake findings (no fix in Phase 0)

- `BENCH_SMALL_UPSTREAM_BODY` product naming smell  
- Cap034 `GET /api/` wire admit path special-case  
- SSE hardcoded `/api/stream*` timeout paths  

```text
ZERO_FAKE_REVIEW = FINDINGS
NO_SMOKE_REVIEW = PASS
```

---

## 12. Phase-1 entry

```text
PHASE_1_FOUNDATIONS_FEASIBLE = CONDITIONAL
PHASE_1_BLOCKERS =
  1. Owner freeze of HYBRID + PROXY_DP_ISOLATION named enum
     (SEPARATE_PROCESS preferred | SAME_PROCESS_STRICT reopen criteria |
      CURRENT_PROCESS_TARGETED_REPLACE as sole path rejected pending owner)
  2. Freeze COMPETITIVE_H1_ACCEPT_OWNER + LISTENER_EXCLUSIVITY
  3. Explicit non-authorization of SDP-P1 resurrection
  4. Correctness/security completeness plan for specialized path (SPPG gaps acknowledged)
  5. HYPER_FALLBACK_ON_COMPETITIVE_H1 admission rule recorded
PHASE_1_PRODUCT_MUTATION_AUTHORIZED = NO
NEXT_WIP_PROPOSAL = V044_COMPETITIVE_FRONTIER_DATAPLANE_FOUNDATIONS
NEXT_WIP_AUTHORIZED = NO
```

### CASE glossary

| CASE | Meaning |
|------|---------|
| CFR-A | Core reconciled; Phase 1 ready (no unresolved Phase-1 blocker) |
| CFR-B | Core reconciled; Phase 1 conditional (owner decision/proof blockers) |
| CFR-C | Architecture contradiction — broader redesign than authorized |
| CFR-D | Insufficient evidence |
| CFR-E | Invalid review (authority/worktree/scope/audit) |

---

## 13. Product value / migration value

Preserving Cap067, FastCGI, Module API, WAF, routing IR maximizes migration value for real deployments. Changing only proxy execution geometry toward hard isolation is the minimum justified evolution for Competitive Frontier.

```text
DOES_THIS_PHASE_INCREASE_MIGRATION_VALUE = YES
IS_CURRENT_MIGRATION_SEQUENCE_CAPABLE_OF_PROTECTING_FRONTIER = NOT_PROVEN
  (sequence designed to protect; no Phase-1+ evidence yet)
```
