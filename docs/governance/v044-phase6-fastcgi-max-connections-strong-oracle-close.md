# V044 Phase 6 — FastCGI max_connections strong oracle close

```text
WIP = V044_PHASE6_FASTCGI_MAX_CONNECTIONS_STRONG_ORACLE_CLOSE
CASE = P6MAX-C
PARENT_TERMINAL = P6ORACLE-C (IMMUTABLE)
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
VALID_RUN = 20260831T012000Z
BINARY_SHA256 = a6107ce62696ac45cdcb692a8df9404ab2752bcca677c2b26ec946347af3dcb0
PRODUCT_MUTATION = NO
WORDPRESS_AUTHORIZED = NO
```

## Purpose

Determine whether CFD `max_connections` has reachable HTTP runtime effect for values >1, without inventing concurrency or mutating architecture.

Parent residual: `MAX_CONNECTIONS_RELOAD = INSUFFICIENT_EVIDENCE` (P6ORACLE-C).

## Case

```text
CASE = P6MAX-C
MEANING = MAX_CONNECTIONS_GT1_SEMANTICALLY_INOPERATIVE
MAX_GT_1_RUNTIME_EFFECT_POSSIBLE = NO
MAX_CONNECTIONS_GT1_SEMANTICS = INOPERATIVE_CURRENTLY
PRODUCT_DEFECT_FOUND = NO
CONFIG_CONTRACT_STATUS = MISLEADING
```

Not A/B (no strong external max causal PASS). Not D (no concurrent sockets on same map that ignore a documented concurrent bound — GEO-2 is shard spill).

## Semantic chain (SOURCE_PROOF)

```text
CONFIG route line parts[4] max_conn
→ cfd-control project.rs → CompiledFcgiPool.max_connections
→ gen IR encode/decode
→ generation publish
→ FcgiPoolMap::ensure_pool → PoolIdle.max_connections
→ take (idle only; no active counter)
→ execute_get BLOCKING on shard thread
→ put: if idle.len() >= max_connections → DISCARD
```

```text
MAX_CONNECTIONS_SCOPE = PER_SHARD × PER_POOL_ID × IDLE_ONLY_CAP
CAN_ONE_SHARD_PROCESS_MULTIPLE_FASTCGI_REQUESTS_CONCURRENTLY = NO
CAN_MULTIPLE_SHARDS_USE_SAME_POOL_ID_CONCURRENTLY = YES (independent maps)
PRIOR_MAX_ORACLE_SERIALIZATION_ROOT_CAUSE = C_FASTCGI_EXECUTION_BLOCKS_SHARD
```

## Config honesty

```text
DOCUMENTED_MAX_CONNECTIONS_CONTRACT (config/ir) =
  "Physical socket cap (busy + idle)"
CFD_ENFORCED =
  idle retention ceiling after put; actives ungated by max
CONFIG_CONTRACT_DRIFT = YES
CONFIG_CONTRACT_STATUS = MISLEADING
OPERATOR_RESOURCE_BOUND_TRUST = NOT_RELIABLE
  # vs process-wide busy+idle reading of the name
IS_CONFIG_PUBLICLY_SUPPORTED = YES  # CFD route line field
```

## Geometry matrix (Netcup VALID 20260831T012000Z)

| GEO | shards | max | CLIENT_CONC | ESTAB peak | PHP overlap | Interpretation |
|-----|--------|-----|-------------|------------|-------------|----------------|
| GEO-1 | 1 | 1 | 6 | **1** | **1** | serial baseline |
| GEO-1 | 1 | 2 | 6 | **1** | **1** | indistinguishable from max=1 |
| GEO-1 | 1 | 6 | 6 | **1** | **1** | indistinguishable from max=1 |
| GEO-2 | 4 | 1 | 8 | **4** | **4** | `SHARD_SPILL_NOT_MAX_CONNECTIONS` |

```text
MAX_INCREASE_RELOAD_CAUSAL = NOT_RUN
MAX_DECREASE_RELOAD_CAUSAL = NOT_RUN
MAX_COUNTERFACTUAL_CONTROL = NOT_APPLICABLE
# No supported HTTP geometry where max_connections changes observable capacity
```

## Preserved parent claims

```text
TTL_RELOAD = STRONG_CAUSAL_PASS
GENERATION_RELOAD = STRONG_CAUSAL_PASS
KEEP_CONN_REGRESSION = PASS (200 / 1 SYN)
```

## Audits

| Agent | Verdict |
|-------|---------|
| Orchestrator | CONDITIONAL_ACCEPT |
| Vertex | LATENT_IDLE_CAP_UNDER_SERIAL_EXECUTE_GET; ARCHITECTURE_AUDIT=PASS |
| Security | OPERATOR_RESOURCE_BOUND_TRUST=FAIL vs busy+idle; P6MAX-D not security MUST_FIX; PASS_WITH_NONBLOCKING |
| Research | PASS_WITH_BOUNDS; GEO design executed |
| GRC | PASS for P6MAX-C / MISLEADING |
| Logic | PASS_WITH_NONBLOCKING_P3; OPEN_PROVEN_PRODUCT_DEFECTS=0 |

## WordPress

```text
MAX_CONNECTIONS_WORDPRESS_BLOCKER = NO
  # if operator accepts serial FastCGI + idle-cap-only semantics explicitly
WORDPRESS_CURRENT_HEAD_QUALIFICATION_FEASIBLE = CONDITIONAL
WORDPRESS_AUTHORIZED = NO
```

## Migration value

```text
DOES_MAX_ORACLE_CLOSE_INCREASE_MIGRATION_VALUE = YES
MIGRATION_VALUE_REASON =
  truthful capacity planning: do not treat CFD max_connections as concurrent
  PHP-FPM socket budget; document idle-only × per-shard × serial execute_get
```

## Evidence / harness

```text
EVIDENCE = .exyonq-local/evidence/phase6-fastcgi-max-connections-strong-oracle-close/20260831T012000Z/
HARNESS = scripts/reality/run-fcgi-max-connections-geometry-netcup.sh
TERMINAL = docs/governance/v044-phase6-fastcgi-max-connections-strong-oracle-close-terminal-block.txt
```

## Recommended next (NOT authorized)

```text
RECOMMENDED_NEXT_WIP =
  OWNER_CHOICE:
    (1) document/rename CFD max_connections as idle_cap honesty close
    (2) authorize async multi-inflight product path for true concurrent max
    (3) WordPress under CONDITIONAL serial/idle-cap qualification
NEXT_WIP_AUTHORIZED = NO
```
