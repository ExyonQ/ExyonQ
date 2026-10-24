# V044 P6D-L1-001 — FastCGI pool reload configuration refresh repair

```text
WIP = V044_P6D_L1_001_FASTCGI_POOL_RELOAD_REFRESH_REPAIR
PARENT = V044_PHASE6D_FASTCGI_RESOURCE_CONCURRENCY_STRESS_QUALIFICATION
PARENT_CASE = P6D-C (IMMUTABLE — do not rewrite)
FINDING_ID = P6D-L1-001
SEVERITY = LOGIC-P1
ADR = docs/adr/040-p6d-l1-001-fcgi-pool-reload-config-refresh.md
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
```

## Root cause

`FcgiPoolMap::ensure_pool` used `HashMap::or_insert_with` only. First-seen
`idle_ttl` / `max_connections` stuck for the lifetime of the map entry when the
same `pool_id` was republished after generation reload.

Netcup VALID P6D `20260830T194007Z`: publish `idle_ms=2000`, sleep 3s, next
requests reused the first-seen ~60s TTL (backend SYN/oracle showed no eviction).

## Chosen design (ADR-040)

```text
CHOSEN_DESIGN = D_REFRESH_POLICY_AND_SWEEP_INVALID_IDLE
NO_NEW_PATTERN_ALTERNATIVE = SUFFICIENT
POOL_POLICY_SSOT = CURRENT_PUBLISHED_GENERATION.CompiledFcgiPool limits
  applied into PoolIdle on ensure_pool
SAME_POOL_ID != SAME_RUNTIME_POLICY_FOREVER
ACTIVE = COMPLETE_THEN_RECONSIDER_ON_PUT
IDLE = immediate TTL/max reconcile on policy change
NEW_GLOBAL_HOT_LOCK = NO
```

## Product mutation

```text
PRIMARY = crates/exyonq-cfd-dataplane/src/fcgi_pool.rs :: ensure_pool
GENERIC_H1_POOL_CHANGED = NO
```

## Harness honesty

Phase6D SUMMARY fields without oracles print `NOT_MEASURED` (never fabricated
`0`/`PASS`). S8 eviction gate uses independent backend reconnect oracles:

- ephemeral local sport change (`ss`)
- and/or `/proc/net/snmp` TcpActiveOpens delta
- tcpdump SYN remains optional (can miss reconnects on lo)

## Evidence

```text
.exyonq-local/evidence/p6d-l1-001-fastcgi-pool-reload-refresh-repair/<RUN_ID>/
```

Historical parent P6D evidence under
`phase6d-fastcgi-resource-concurrency-stress-qualification/` is frozen.

## Regression

- Component: TTL↓/↑, max↓/↑, same pool_id, shrink eventual, map stability
- Netcup: S8 idle eviction + KEEP_CONN long reuse; S9 churn
- P6C activate_before_begin paths preserved
- Residual `FCGI_POST_PROBE_TO_FIRST_WRITE_MICRO_WINDOW` unchanged (future review)

## Notion facts (terminal)

Fill from terminal block after Netcup close — do not invent PASS for unmeasured
fields.
