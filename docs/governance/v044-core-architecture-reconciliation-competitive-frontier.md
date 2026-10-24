# Governance — Core architecture reconciliation (Competitive Frontier)

```text
WIP = V044_CORE_ARCHITECTURE_RECONCILIATION_FOR_COMPETITIVE_FRONTIER
PHASE = CORE_REVIEW_PHASE_0
MODE = READ_ONLY_CORE_ARCHITECTURE_REVIEW
CASE = CFR-B
PRODUCT_MUTATION = NO
ARCHITECTURE_MUTATION = NO
DOCS_MUTATION = YES_ONLY_FOR_PHASE_0_RECONCILIATION
PHASE_1_AUTHORIZED = NO
NEXT_PHASE_AUTOSTART = NO
CANONICAL_MERGE = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
PUBLIC_MIGRATION_CLAIMS = FORBIDDEN
```

Architecture: [`../architecture/v044-core-architecture-reconciliation-competitive-frontier.md`](../architecture/v044-core-architecture-reconciliation-competitive-frontier.md)  
Performance budget: [`../performance/v044-core-architecture-performance-budget-contract.md`](../performance/v044-core-architecture-performance-budget-contract.md)  
Evidence: `.exyonq-local/evidence/core-architecture-reconciliation/20260828-122859/`

Orchestrator admission: **CONDITIONAL ACCEPT** (docs-only Phase 0).

---

## Binding outcomes

```text
EXTREME_PLUS25_MINUS10 = CLOSED
COMPETITIVE_FRONTIER_TARGET = RPS+9.3 / CPU+22.9 / MEM+17.3 / P95+16.5 / P99+14.1
SPPG = ARCHITECTURE_TARGET_REFERENCE_NOT_SHIPPING_BASELINE
SDP_P1_PRODUCTIZATION = CLOSED (IL-F)
PXDP = HOLD
CAP067_REMOVAL = NOT_AUTHORIZED
TARGET_PROCESS_MODEL = HYBRID (owner confirmation required)
PROXY_DP_ISOLATION_PREFERRED = SEPARATE_PROCESS
  # named enum — SPPG letter A ≠ CFR SEPARATE_PROCESS
SPLIT_PLANE_BOUNDARY = RECOMMENDED_PENDING_OWNER_FREEZE
MIGRATION_SEQUENCE = FROZEN
PHASE_1_FOUNDATIONS_FEASIBLE = CONDITIONAL
NEXT_WIP_PROPOSAL = V044_COMPETITIVE_FRONTIER_DATAPLANE_FOUNDATIONS
NEXT_WIP_AUTHORIZED = NO
LOGIC_AUDIT = FAIL_PROVEN_FINDINGS (doc narrows applied; OPEN_P0=0 OPEN_P1=ADR-letter mitigated)
```

Even if later CASE upgrades to CFR-A, **do not start Phase 1** without a new owner admission message.

---

## Ambient worktree dirt

```text
AMBIENT_DIRT = YES (~77 paths at entry)
CLASSIFICATION = OUT_OF_SCOPE
ACTION = do not stage, revert, absorb, or fold into Phase 0 conclusions
```

Includes prior WIP docs/scripts and uncommitted product `M` files (waf, proxy, etc.). Phase 0 docs are additive only under architecture/governance/performance allowlist.

---

## HOLDs carried forward

```text
REQUEST_BODY_RELAY = HOLD until Phase 4 entry
PXDP_P6 = HOLD
PXDP_FEATURE_EXPANSION = HOLD
H2_H3_EXPANSION = HOLD_IF_EXECUTION_DEPENDENT
WEBSOCKET_CONNECT_ON_SPECIALIZED_DP = HOLD (initial)
MULTI_CONCURRENCY_SWEEP = HOLD
WORKER_DEFAULT_CHANGE = NOT_AUTHORIZED
P4_MICRO_OPTIMIZATION_CAMPAIGN = STOPPED
EXYONQ_SDP_P1 = DEFAULT_OFF / do not resurrect
```

---

## Security / correctness non-tradeable

```text
PERFORMANCE_WIN_WITH_SECURITY_FAIL = FAIL
WAF_WEAKENING_FORBIDDEN = YES
NO_SMOKE_AS_EVIDENCE = YES
SPPG_NOT_PRODUCT_PROOF = YES
```

---

## Lineage

GA-B → EP-C → SD-B → SDP-P1/SP1-C → IL-F → SPPG-C → **CFR Phase 0 (CFR-B)** → (owner review) → Phase 1 only if newly authorized
