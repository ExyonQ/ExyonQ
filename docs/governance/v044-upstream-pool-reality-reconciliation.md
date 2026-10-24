# V044 — Upstream pool reality reconciliation

```text
WIP = V044_UPSTREAM_POOL_REALITY_RECONCILIATION
CASE = UPRR-C
ROOT_CAUSE_CLASS = UPRR-R6 (+ secondary UPRR-R3 if Phase-2 C5 still treated as live REQUIRED)
PRODUCT_MUTATION = NO
PHASE_6_AUTHORIZED = NO
```

## Chronology (do not erase)

| Era | WHAT_WAS_CLAIMED | WHAT_WAS_ACTUALLY_PROVEN | WHAT_IS_TRUE_NOW |
|-----|------------------|-------------------------|------------------|
| Phase 2 terminal | Shard-local upstream KA pool + poison discard; client + upstream keepalive | Pool **API + put path existed**; CFD-L06 proves a socket was pooled then reused (stale→502). **Hit-rate / connect≪requests not proven** | Historical claim **partially true** (mechanism), **overstated** as unqualified “KA pools delivered” |
| Phase 4 bodies / framing | Fail-closed trailing-byte policy | `POOL_INTERACTION = STRONGLY_SUPPORTED (no upstream reuse after response)`; security: `UPSTREAM_TRAILING_BYTES = POISON` | Intentional **disable of put** for smuggling/framing safety |
| Phase 5 / Profile A / EXT-A | Competitive frontier (CPU/RPS) | Client keepalive on CFD path; OpenCode EXT-A: connect-per-request upstream | **Upstream reuse = NO**; `UpstreamPool::put` unreachable; `take` always misses |

**WHY_THE_STATE_CHANGED:** Phase-4 security fail-closed (trailing-byte probe race) superseding Phase-2 pool return — **documented intent**, not accidental dead wiring.

## Current source truth (Profile A / competitive H1 CFD path)

> **Supersession (non-erasing):** WIP `V044_SAFE_H1_UPSTREAM_REUSE_RESTORATION` (URR-A) re-enabled **safe** Hybrid-F put under ADR-021 after this reconciliation froze the discard-only era. The table below remains the **UPRR-era** snapshot. Live post-repair truth: see `docs/governance/v044-safe-h1-upstream-reuse-restoration.md`.

| Question | Answer (UPRR-era) | Evidence |
|----------|-------------------|----------|
| Client keepalive | **YES** | `finish_response` → `reset_for_keepalive` when `!client_close` |
| Upstream keepalive / reuse | **NO** (at UPRR freeze) | Always `UpstreamPool::discard` after response |
| `UpstreamPool::take` reachable | **YES** | `attach_upstream` / `pool.take` |
| Pool hit possible in practice | **NO** (at UPRR freeze) | Nothing calls `put` → idle map empty |
| `UpstreamPool::put` reachable | **NO** (at UPRR freeze) | Definition only; `#[allow(dead_code)]` |
| Success disposition | **CLOSE / DISCARD** (at UPRR freeze) | `finish_response` ignores `reuse_ok` |
| Error disposition | **DISCARD** | `discard_upstream` / poison paths |

Primary files (UPRR-era snapshot):

- `crates/exyonq-cfd-dataplane/src/upstream_pool.rs` — at UPRR freeze: `put` dead; Phase-4 never called
- `crates/exyonq-cfd-dataplane/src/shard.rs` — at UPRR freeze: `take` on attach; `finish_response` always discard

Post-URR-A repair paths (do not erase UPRR rows above): Hybrid-F `put`/`take` with dual probe + idle READABLE — see restoration doc.

## Contract class

```text
UPSTREAM_REUSE_CONTRACT_CLASS = INTERNAL_OPTIMIZATION / OPTIONAL_REENABLE_CANDIDATE
  not a HTTP correctness invariant for current fail-closed path
  Phase-2 C5 / “REQUIRED upstream KA” language is STALE relative to Phase-4+
```

```text
SUPPORTED_PRODUCT_DEFECT (correctness) = NO
DOC_RECONCILIATION_REQUIRED = YES
PRODUCT_REPAIR_REQUIRED = NO   # mandatory correctness
PRODUCT_REPAIR_RECOMMENDED = OPTIONAL future WIP if restoring C5 under safe idle probe
```

## Benchmark impact

Asymmetry is **anti-ExyonQ** (extra upstream connect work vs rivals that reuse). Therefore:

| Claim | Impact |
|-------|--------|
| P5CPU-A | **UNCHANGED** |
| EXT-A | **UNCHANGED** |
| CPU / RPS frontier | **UNCHANGED** (conservative vs rivals) |
| Pingora comparison | **UNCHANGED** |

Do **not** invalidate proven frontiers merely because ExyonQ reconnects upstream.

## Migration value

```text
DOES_CURRENT_NON_REUSE_REDUCE_MIGRATION_VALUE = PARTIAL
  # Operators from NGINX/OLS/HAProxy expect upstream KA; CFD does not provide it today.
DOES_THIS_RECONCILIATION_INCREASE_MIGRATION_VALUE = YES
  # Removes false capability belief before Phase 6.
```

## Resource risk (architecture, not measured campaign)

| Risk | Level |
|------|-------|
| Upstream connection pressure | **MEDIUM** (connect≈request) |
| Ephemeral ports / TIME_WAIT | **MEDIUM** under high concurrency |
| Upstream FD churn | **MEDIUM** |
| Distant upstream / TLS handshake cost | **HIGH** if TLS upstream later without reuse |
| Overall RESOURCE_RISK | **MEDIUM** |

```text
EXPECTED_PERFORMANCE_GAIN_IF_REENABLED = UNKNOWN_UNTIL_MEASURED
```

## Documentation inventory (selected)

| Doc | Classification |
|-----|----------------|
| `docs/governance/v044-competitive-frontier-real-h1-routing-headers.md` (“KA pools”) | **OVERSTATED / HISTORICALLY_TRUE_CURRENTLY_FALSE** on live reuse |
| `docs/architecture/v044-competitive-frontier-real-h1-routing-headers.md` §6 | Scaffold **CURRENTLY_TRUE**; reuse **CURRENTLY_FALSE** |
| `docs/performance/v044-competitive-frontier-request-bodies-backpressure-budget.md` | **CURRENTLY_TRUE** (no reuse after response) |
| `docs/security/v044-h1-framing-smuggling-backpressure.md` | **CURRENTLY_TRUE** (poison / fail-closed) |

## ADR-021 pre-repair (analysis only — not implemented)

See `docs/governance/v044-adr021-upstream-pool-pre-repair-analysis.md`.

Alternatives A–D compared; **no code change in this WIP**.

## Evidence

`.exyonq-local/evidence/upstream-pool-reality-reconciliation/uprr-20260829T223058Z/`

Independent logic audit: **PASS_WITH_FINDINGS** (OPEN_P0=0, OPEN_P1=0, OPEN_P2=1 doc/contract drift).

## Notion reconciliation facts (owner)

1. Phase 2 delivered pool **structure** + historical `put` path (CFD-L06), not proven hit-rate economics.
2. Since Phase 4, CFD **always discards** upstream after response (`put` never called).
3. Client keepalive ≠ upstream keepalive.
4. Do not market “upstream KA pools” as a live CFD performance capability.
5. P5CPU-A / EXT-A / Pingora claims remain standing; asymmetry is conservative for ExyonQ.
6. Optional future WIP: safe idle validation + re-enable `put` under ADR-021 — **not authorized here**.
