# V044 — Required agent set reconciliation

```text
WIP = EXYONQ_REQUIRED_AGENT_SET_RECONCILIATION
PRODUCT_MUTATION = NO
PARENT_WIP = V044_SAFE_H1_UPSTREAM_REUSE_RESTORATION
PARENT_STATUS_AT_BLOCK = BLOCKED_REQUIRED_AGENT_OR_RULE_UNAVAILABLE / URR-E
CASE = AGENTSET-B
```

## Why the parent WIP stopped

`V044_SAFE_H1_UPSTREAM_REUSE_RESTORATION` correctly refused silent substitution when these mandatory agents were absent:

| Required | Status before this WIP |
|----------|------------------------|
| `exyonq-orchestrator` | present |
| `exyonq-grc-engineer` | present |
| `exyonq-logic-auditor` | present |
| `exyonq-vertex-architect` | **missing** |
| `exyonq-security-engineer` | **missing** |
| `exyonq-research-engineer` | **missing** |

Nearest existing agents (`exyonq-architecture-contract`, `exyonq-tech-lead`, `exyonq-security`, `exyonq-security-compiler`, `exyonq-networking-proxy`, `exyonq-core-systems`) were **not** authorized as substitutes. That STOP remains valid historical evidence (URR-E).

## What this WIP created

Exact paths:

- `.cursor/agents/exyonq-vertex-architect.md`
- `.cursor/agents/exyonq-security-engineer.md`
- `.cursor/agents/exyonq-research-engineer.md`

Each has a **distinct charter** (not a renamed clone):

| Agent | Authority | Explicit non-overlap |
|-------|-----------|----------------------|
| Vertex architect | Execution-path architecture + ADR-021 gate + dataplane invariants | ≠ architecture-contract; ≠ tech-lead |
| Security engineer | Reachable-path security ACCEPT/REJECT; dual-boundary reuse; adversarial evidence | ≠ security (invariants); ≠ security-compiler (merge gate) |
| Research engineer | Evidence grades, archaeology, CLAIM≤EVIDENCE | ≠ performance-validation; ≠ GRC |

Hardening in the same WIP (agents + orchestrator binding only):

- SoD: `IMPLEMENTATION_AGENT ≠ VERTEX/SECURITY_ACCEPT_ISSUER` and `≠ FINAL_LOGIC_AUDITOR`
- `CONDITIONAL_ACCEPT ≠ MUTATION_AUTHORIZED` (re-verdict required)
- `SECURITY ∈ {REJECT, BLOCKED} → PRODUCT_MUTATION_FORBIDDEN`
- `OWNER_REVIEW` cannot clear security REJECT/BLOCKED without fresh `SECURITY_ACCEPT` + written concurrence
- Orchestrator §13.1 binds the six mandatory roles with **no silent substitution**

## Interaction contract

```text
exyonq-orchestrator          → WIP coordination / admission (cannot override hard REJECT/BLOCKED)
exyonq-vertex-architect      → architecture acceptance
exyonq-security-engineer     → security acceptance
exyonq-research-engineer     → research / evidence reconstruction
exyonq-grc-engineer          → governance / evidence contract
exyonq-logic-auditor         → independent adversarial terminal auditor

HARD_REJECT_CONFLICT → OWNER_REVIEW
OWNER_REVIEW ≠ SECURITY_ACCEPT
OWNER_REVIEW_CLEARS_SECURITY_REJECT_OR_BLOCKED = FORBIDDEN
SILENT_OVERRIDE = FORBIDDEN
ZERO_FAKE = ABSOLUTE
NO_SMOKE = ABSOLUTE
ANTI_KOBAYASHI = REQUIRED
```

## Validation (this WIP)

| Gate | Result | Notes |
|------|--------|-------|
| Orchestrator agent-set acceptance | PASS | Six mandatory names resolve; no silent substitution |
| GRC agent-set review | PASS | Nonblocking: 106/107 maps still deferred for broad auto-invoke |
| Logic auditor agent-set audit | PASS_WITH_FINDINGS | P1 OWNER_REVIEW supersession **CLOSED**; residual nonblocking: research BLOCKED-only taxonomy; GRC/logic charters lack mirrored SoD lines |
| Silent substitution | NO | Exact agent names created |
| Zero-fake / no-smoke / anti-Kobayashi | PASS | Explicit in all three charters |

`CASE = AGENTSET-B` because 106/107 specialist-map auto-wiring and peer-charter mirrors remain nonblocking qualifications for parent reissue checklists — not because agents are invalid.

## Evolution memory

```text
CANONICAL_EVOLUTION_MEMORY_PATH = .cursor/evolution_memory.md
EVOLUTION_MEMORY_REQUIRED = YES
EVOLUTION_MEMORY_STATUS = PRESENT
```

Authority: `.cursor/rules/evolution_memory.mdc`, `.cursor/rules/dspy-gepa-darwin.mdc`.

Alternate spellings (`evolution-memory.md`) were **not** found as canonical. A prior parent-block report of “ABSENT” was incorrect relative to this tree’s canonical path (file present). No AEY import. No second memory file invented.

## Parent product WIP

```text
PARENT_PRODUCT_WORK_STARTED = NO
PARENT_WIP_CAN_BE_REISSUED = YES
```

Owner must **reissue** `V044_SAFE_H1_UPSTREAM_REUSE_RESTORATION` separately. This WIP does **not** start upstream reuse repair, ADR-021 product mutation, Netcup, or Phase 6.

## Chronology preserved

- UPRR-C / UPRR-R6 (upstream pool reality) remains standing.
- URR-E block for missing agents remains standing as correct process.
- No claim that safe H1 upstream reuse has been restored.
