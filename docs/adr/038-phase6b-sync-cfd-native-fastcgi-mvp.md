# ADR 038 — Phase 6B sync CFD-native FastCGI MVP

```text
ADR = 038
TITLE = Phase 6B sync CFD-native FastCGI MVP
STATUS = ACCEPTED
DATE = 2026-08-30
WIP = V044_PHASE6B_SYNC_CFD_NATIVE_FASTCGI_MVP
PARENT = V044_PHASE6A_FASTCGI_MODULE_ARCHITECTURE_ADMISSION (P6A-C)
OWNER_ARCHITECTURE_DECISION = SYNC_CFD_NATIVE_FASTCGI_WIRE_MVP
NOTE = Owner package named this decision "ADR-021 Phase6B"; formal ADR-021 already
       binds safe H1 upstream reuse — this decision is ADR-038 to avoid collision.
```

## Context

Phase 6A proved FastCGI exists on the product Tokio/Hyper path and is absent from the competitive CFD dataplane. Owner rejects permanent “CFD reject-only + product FastCGI forever” as the Phase-6 end state. Target: **real FastCGI execution inside CFD** without Tokio, Hyper, per-request IPC, dual accept, or transparent fallback.

## Options

| ID | Option | Verdict |
| --- | --- | --- |
| A | Permanent product FastCGI + CFD reject | Rejected as **final** Phase-6 architecture (allowed only as interim fail-closed during rollout) |
| B | Per-request CFD→product handoff | **REJECT** — violates per-request handoff / cross-process RPC invariants |
| C | Embed Tokio/Hyper FastCGI into CFD | **REJECT** — violates Tokio=0 / Hyper=0 |
| **D** | Sync CFD-native FastCGI using portable wire logic | **ACCEPT** |
| E | Greenfield FastCGI rewrite | **REJECT** — unjustified; wire/commit/params already portable |
| F | Other | None justified by source |

## Decision

**D — Sync CFD-native FastCGI wire MVP**

1. Extract/reuse Tokio-free FastCGI codec, PARAMS, commit/retry, and caps into `exyonq-fastcgi-wire`.
2. Extend immutable CFD generation with `BackendKind::{Proxy|Fastcgi|Reject}` and `CompiledFcgiPool`.
3. CFD shard executes FastCGI synchronously (`std`/`mio`-compatible blocking I/O) on matched routes.
4. Shard-local FastCGI pool (no global hot lock); KEEP_CONN only after clean END_REQUEST + empty trailing-byte validation (SG-FCGI-05).
5. Product-plane Tokio FastCGI remains for **product listeners only** — never transparent fallback after CFD accept.
6. Module API / WordPress / WASM deferred.

## Invariants preserved

```text
TOKIO_IN_CFD = 0
HYPER_IN_CFD = 0
CONTROL_PLANE_RPC_PER_REQUEST = 0
CROSS_PROCESS_RPC_PER_REQUEST = 0
DUAL_ACCEPT = FORBIDDEN
TRANSPARENT_FALLBACK_AFTER_DP_SIDE_EFFECT = FORBIDDEN
NEW_GLOBAL_HOT_LOCK = FORBIDDEN
```

## Consequences

- Generation schema / route-table magic bump (fail-closed for old incompatible CFD).
- Product FastCGI crate not linked into CFD as a whole (would pull Tokio/Hyper).
- Terminal proof requires real PHP-FPM (Netcup amd64); scripted peers are non-terminal.
- FastCGI performance frontier not claimed in P6B.

## POSTHOC reconciliation — terminal seal (2026-08-30)

```text
RECONCILIATION_CLASS                         = POSTHOC
SEAL_DOC                                     = docs/governance/v044-phase6b-fastcgi-mvp-terminal-seal.md
PHASE6B_PRE_MUTATION_ARCHITECTURE_REVIEW     = FAIL
PRE_MUTATION_PROOF                           = MISSING
ADR021_GOVERNANCE_REQUIREMENT                = INCOMPLETE
ADR_COLLISION_REASON                         = ADR021_IDENTIFIER_ALREADY_ASSIGNED
FORMAL_ADR_FOR_PHASE6B                       = 038
ADR021_REMAINS                               = Safe H1 upstream reuse restoration
OWNER_TEMPLATE_LABEL_ADR021_PHASE6B          = NON_AUTHORITATIVE_HISTORICAL_NAME_ONLY
HISTORICAL_STATUS_ACCEPTED_ERASED            = NO
PRE_MUTATION_ACCEPT_REWRITTEN_AS_EXISTING    = FORBIDDEN
```

Option **D** remains the recorded architecture decision (`STATUS = ACCEPTED`). That records the chosen architecture; it does **not** prove a complete ADR-021-discipline pre-mutation analysis (ownership, lifecycle SM, dual-boundary probes, Arc/Mutex budget, rollback, SSOT, …) existed before mutation.

Post-implementation Netcup evidence (seal RUN `20260830T142958Z`) may claim causal KEEP_CONN reuse under an independent SYN oracle; that evidence does not rewrite pre-mutation Accept history.

Field gaps vs ADR-021 governance analysis contract (binding surface = this ADR file): ownership, mutability, concurrency detail, lifecycle SM, error/poison semantics, hot-path alloc analysis, Arc/Mutex/RwLock/atomics/clones/Box-dyn/lifetimes/RAII/newtype, cleanup, SSOT, rollback, full security consequences — **MISSING** at pre-mutation time. `NO_NEW_PATTERN_ALTERNATIVE` not stated as the required discipline token (Option A is closest).
