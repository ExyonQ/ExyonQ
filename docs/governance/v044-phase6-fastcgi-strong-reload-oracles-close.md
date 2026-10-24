# V044 Phase 6 — FastCGI strong reload oracles close

```text
WIP = V044_PHASE6_FASTCGI_STRONG_RELOAD_ORACLES_CLOSE
CASE = P6ORACLE-C
PARENT = P6PREWP-B (IMMUTABLE)
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
VALID_RUN = 20260831T005500Z
BINARY_SHA256 = a6107ce62696ac45cdcb692a8df9404ab2752bcca677c2b26ec946347af3dcb0
PRODUCT_MUTATION = NO
WORDPRESS = NO
```

## Purpose

Replace P6PREWP survival-only stamps:

```text
TTL_RELOAD = SURVIVAL_AFTER_PUBLISH
MAX_CONNECTIONS_RELOAD = SURVIVAL_AFTER_PUBLISH
GENERATION_RELOAD = SURVIVAL_AFTER_PUBLISH
```

with strong causal runtime oracles where evidence allows.

```text
RELOAD_ACCEPTED ≠ RELOAD_EFFECT_CAUSALLY_PROVEN
```

## Case

```text
CASE = P6ORACLE-C
# TTL + GENERATION strong; MAX remains INSUFFICIENT_EVIDENCE
FASTCGI_STRONG_RELOAD_ORACLE_BLOCKERS = 1  # MAX
```

Not A/B: harness requires all three tracks `STRONG_CAUSAL_PASS`.

## Track results (VALID 20260831T005500Z)

| Track | Claim | Oracles |
|-------|--------|---------|
| **TTL** | `STRONG_CAUSAL_PASS` | Warm **after** short-TTL publish; SPORT change; same-sleep long-TTL counterfactual reuse; increase bounds; multi 60→2→30→1 |
| **MAX** | `INSUFFICIENT_EVIDENCE` | Concurrent ESTAB stays 1 under max=2 and max=6 — serial blocking `execute_get` + KEEP_CONN ⇒ `idle.len()≤1` on one shard. Not a proven product defect under ADR-040; strong external multi-idle oracle needs authorized async multi-inflight (PRODUCT_MUTATION=NO here). |
| **GEN** | `STRONG_CAUSAL_PASS` | Dual retained docroots `GENMARK_A…D`; post-publish admissions; connection SPORT transition; multi; failed `script_suffix` publish EFFECT=NO |
| **KEEP_CONN** | `PASS` | 200 requests / 1 backend SYN |

## Generation model

```text
CONFIG_GENERATION          = published generation_id in gen-dir
REQUEST_ADMITTED_GENERATION = shard local_gen_id at request accept
POOL_POLICY_GENERATION     = CompiledFcgiPool snapshot applied via ensure_pool
CONNECTION_GENERATION      = IdleConn.generation tag; flush_generation on gen advance
```

## Source / binary identity

```text
SOURCE_PROVENANCE = PASS_BOUNDED
ATTRIBUTABLE_SOURCE_METHOD = B1_DETACHED_WORKTREE_PLUS_BOUNDED_OVERLAY (parent) + harness/docs overlay this WIP
BASE_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
BASE_TREE = ac059750141ee48f141c543cccbd641652bf49fb
BINARY_IDENTITY = PASS (local build sha == remote sha)
PRODUCT_MUTATION = NO
HARNESS_MUTATION = YES
```

## Run disclosure

| RUN_ID | Disposition |
|--------|-------------|
| 20260831T002100Z | INVALID — `GEN_ID` stuck in `$(next_gen)` subshell |
| 20260831T003600Z | INVALID — cargo not on PATH under bare nohup |
| 20260831T003700Z | ABORTED — MAX shrink curl hang; TTL partial only |
| 20260831T004800Z | SUPERSEDED — `GENMARK_{$mark}` bash braces bug |
| **20260831T005500Z** | **VALID** |

## Audits

| Agent | Verdict |
|-------|---------|
| Orchestrator | CONDITIONAL_ACCEPT (pre) |
| Vertex | ARCHITECTURE_AUDIT=PASS |
| Security | PASS_WITH_NONBLOCKING (post); checklist TTL/GEN met; MAX bound |
| Research | PASS (oracle methodology) |
| GRC | PASS for P6ORACLE-C; blockers=1 |
| Logic | PASS_WITH_NONBLOCKING_P3 (2 harness hygiene P3; 0 product defects) |

## Claim ceiling

```text
CLAIM_CEILING = BOUNDED_LINUX_AMD64_CAUSAL_RELOAD_ORACLES
≠ VERIFIED_REAL_PRODUCTION
≠ dual-arch shipping admission
ORACLE_ARM64_REQUIRED = NO_BY_DEFAULT (evidence-strength close only)
```

## WordPress

```text
WORDPRESS_AUTHORIZED = NO
WORDPRESS_CURRENT_HEAD_QUALIFICATION_FEASIBLE = CONDITIONAL_WITH_MAX_ORACLE_GAP
RECOMMENDED_NEXT_WIP = ASYNC_MULTI_INFLIGHT_FCGI_OR_AUTHORIZED_MAX_ORACLE_PRODUCT_PATH | CURRENT_HEAD_WORDPRESS_WITH_MAX_QUALIFICATION
NEXT_WIP_AUTHORIZED = NO
```

## Evidence / harness

```text
EVIDENCE = .exyonq-local/evidence/phase6-fastcgi-strong-reload-oracles-close/20260831T005500Z/
HARNESS = scripts/reality/run-fcgi-pool-reload-causal-oracles-netcup.sh
TERMINAL = docs/governance/v044-phase6-fastcgi-strong-reload-oracles-close-terminal-block.txt
```

## Migration value

```text
DOES_STRONG_RELOAD_ORACLE_CLOSE_INCREASE_MIGRATION_VALUE = PARTIAL
# TTL + generation reload now causally trusted; pool max_connections still evidence-weak externally
```
