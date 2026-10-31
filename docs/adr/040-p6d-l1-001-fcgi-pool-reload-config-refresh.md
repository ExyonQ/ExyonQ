# ADR 040 — Phase 6D FastCGI pool reload configuration refresh repair

```text
ADR = 040
TITLE = Phase 6D FastCGI pool reload configuration refresh repair
STATUS = ACCEPTED
DATE = 2026-08-30
WIP = V044_P6D_L1_001_FASTCGI_POOL_RELOAD_REFRESH_REPAIR
PARENT_WIP = V044_PHASE6D_FASTCGI_RESOURCE_CONCURRENCY_STRESS_QUALIFICATION
PARENT_CASE = P6D-C
FINDING_ID = P6D-L1-001
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
PRODUCT_MUTATION_AUTHORIZED = YES
ORCHESTRATOR_ADMISSION = CONDITIONAL_ACCEPT (L2)
VERTEX_PRE_MUTATION = PASS
SECURITY_PRE_MUTATION = PASS (LIVE_COMPILED_LIMITS_SSOT)
GRC_PRE_MUTATION = PASS
RESEARCH_BIND = PROVEN (VALID P6D 20260830T194007Z; BINARY 926c6289…)
PRODUCT_MUTATION_STARTED_UTC = 2026-08-30T20:31:00Z
NETCUP_REPAIR_RUN = 20260830T204920Z
REPAIR_BINARY_SHA256 = 3eb7a478233da06e836b2e3313ebc3151ce77bb4253e6fd9fcb6de4d076947bc
DO_NOT_REUSE = ADR-039
```

## PROBLEM_DEFINITION

`FcgiPoolMap::ensure_pool` creates a `PoolIdle` via `HashMap::or_insert_with` and **never refreshes** `idle_ttl` / `max_connections` when the same `pool_id` is presented again after a generation publish with new `CompiledFcgiPool` limits.

Consequently the first-seen runtime policy becomes a permanent de-facto SSOT while the published generation claims newer limits.

Netcup VALID evidence (`20260830T194007Z`, S8): publish `idle_ms=2000`, sleep 3s, next requests → backend SYN=0 under sticky ~60s TTL.

## CURRENT_BEHAVIOR

```text
ensure_pool(pool):
  by_id.entry(pool.id).or_insert_with(|| PoolIdle{max, ttl, idle:[]})
  # if entry exists → NO-OP (stale max/ttl remain)

take/put:
  ensure_pool(pool)
  enforce idle.idle_ttl / idle.max_connections  # sticky fields

flush_generation(keep):
  filter idle by generation tag only
  # does NOT refresh policy
```

## SUPPORTED_CONTRACT

```text
CURRENT_COMPILED_GENERATION
  → CURRENT_FASTCGI_POOL_POLICY (CompiledFcgiPool.idle_timeout_ms, max_connections)
  → runtime enforcement on that pool_id

SAME_POOL_ID != SAME_RUNTIME_POLICY_FOREVER

OLD_IDLE_TTL_MUST_NOT_REMAIN_AUTHORITATIVE = YES
OLD_MAX_CONNECTIONS_MUST_NOT_REMAIN_AUTHORITATIVE = YES
KEEP_CONN / causal reuse PRESERVED under stable policy
RECONNECT_EVERY_REQUEST_FAKE_FIX = FORBIDDEN
P6C B_PRE_BEGIN_ALWAYS_PROBE = PRESERVED
FCGI_POST_PROBE_TO_FIRST_WRITE_MICRO_WINDOW = OUT_OF_SCOPE (ledger preserved)
```

## TARGET_BEHAVIOR

```text
ensure_pool(pool):
  create PoolIdle if missing
  ELSE apply live max_connections + idle_ttl as ONE coherent snapshot
  then reconcile idle list:
    discard idle with age > new idle_ttl
    while idle.len() > new max: discard excess idle
  map entry count for same pool_id stays 1 (no unbounded growth)

take/put:
  ensure_pool first → always see current policy
  active requests NOT killed by ensure_pool
  put: probe → enforce NEW max → else discard
```

## ALTERNATIVES

| ID | Design | Verdict |
| --- | --- | --- |
| A | Mutate config in place only | **INSUFFICIENT alone** — misses immediate idle reconcile on TTL/max shrink |
| B | Replace entire `PoolIdle` on change | ACCEPTABLE fallback; drops reusable idle on any change; heavier than needed |
| C | Version pools by gen/config identity | **REJECT** — new keying pattern; map growth risk |
| **D** | Refresh policy + immediately evict now-invalid idle | **ACCEPT (CHOSEN)** |
| E | Lazy refresh only at take/put | WEAKER — delays max reconcile; owner requires idle reconcile on policy change |
| F | Immutable gen-bound handles + drain | **REJECT** — new pattern |
| G | Smaller A-only extension | **REJECT as terminal** |

```text
CHOSEN_DESIGN = D_REFRESH_POLICY_AND_SWEEP_INVALID_IDLE
NO_NEW_PATTERN_ALTERNATIVE = SUFFICIENT
IS_A_NEW_PATTERN_ACTUALLY_REQUIRED = NO
FALLBACK_IF_D_FAILS_PROOF = B
```

## OWNERSHIP

| Object | Owner | Notes |
| --- | --- | --- |
| `FcgiPoolMap` | Shard thread only (`&mut`) | No Arc/Mutex; no cross-thread share |
| `PoolIdle` policy fields | Updated only via `ensure_pool` from presented `CompiledFcgiPool` | SSOT application point |
| Idle streams | `PoolIdle.idle` until take | Exclusive |
| Active streams | Request path (`fcgi_exec`) until put/discard | Not in map; ensure_pool cannot terminate them |
| Generation tag on idle | `IdleConn.generation` | Orthogonal to policy refresh; `flush_generation` unchanged |

## MUTABILITY

`PoolIdle.{max_connections,idle_ttl}` are mutable caches of the **last applied** `CompiledFcgiPool` snapshot for that `pool_id`. They must be overwritten whenever `ensure_pool` is presented a (possibly different) compiled pool for the same id.

## CONCURRENCY

CFD dataplane is shard-local single-threaded for a given `FcgiPoolMap`. Reload/`ensure_pool` and take/put cannot race across threads on the same map. Interleaving is sequential on the shard loop. No new locks.

```text
NEW_GLOBAL_HOT_LOCK = NO
NO_DOUBLE_OWNERSHIP = YES (idle XOR active exclusive)
NO_CONFIG_TORN_STATE = YES (snapshot assign then sweep)
NO_ACTIVE_IDLE_ALIAS = YES
NO_BOUND_BYPASS = YES (put enforces new max; excess idle trimmed)
NO_STALE_POLICY_PERMANENCE = YES (refresh on ensure_pool)
```

## LIFECYCLE_STATE_MACHINE

```text
MISSING_POOL --ensure_pool--> POOL_WITH_POLICY(idle=[])
POOL_WITH_POLICY --ensure_pool(new policy)--> POOL_WITH_POLICY'
  (fields replaced; idle reconciled)
IDLE --take(probe+gen+ttl)--> CANDIDATE --activate_before_begin--> ACTIVE
ACTIVE --put(probe+max)--> IDLE | DISCARD
ACTIVE --error/poison--> DISCARD
GEN_MISMATCH idle --> DISCARD (flush_generation / take)
```

## GENERATION_AUTHORITY / POOL_POLICY_GENERATION_MODEL

```text
POOL_POLICY_GENERATION_MODEL =
  Request presents CompiledFcgiPool from the current published generation projection.
  PoolIdle policy fields track the latest Applied CompiledFcgiPool for that pool_id
  (via ensure_pool on take/put/connect paths that call it).
  IdleConn.generation still gates reuse of sockets across generation flushes.
  A G1 request must not mutate G2 policy: it only presents its own CompiledFcgiPool;
  if G2 already applied, G1-shaped ensure_pool would re-apply G1 limits only if a
  stale projection were still served — generation publish + route projection prevent
  that on the supported path. flush_generation discards wrong-gen idle sockets.
```

## ACTIVE_CONNECTION_SEMANTICS / ACTIVE_CONNECTION_POLICY_ON_RELOAD

```text
ACTIVE_CONNECTION_POLICY_ON_RELOAD =
  COMPLETE_ACTIVE_THEN_RECONSIDER_ON_PUT

EXISTING_ACTIVE_CONNECTIONS =
  Not resident in PoolIdle; ensure_pool does NOT terminate them.
  In-flight requests complete; connection reconsidered vs CURRENT policy on put
  (probe + new max). If over-cap at put → discard (no silent over-cap idle).
```

## IDLE_CONNECTION_SEMANTICS / IDLE_CONNECTION_POLICY_ON_RELOAD

```text
IDLE_CONNECTION_POLICY_ON_RELOAD =
  RECONCILE_IMMEDIATELY_AGAINST_NEW_POLICY

EXISTING_IDLE_CONNECTIONS =
  On ensure_pool after policy assign:
    discard if now.duration_since(idle_since) > new idle_ttl
    while len > new max: discard excess (LIFO pop)
  take continues to enforce ttl/gen/probe with refreshed fields.

DEADLINE_MODEL = idle_since + CURRENT_POLICY.idle_ttl
OLD_IDLE_TTL_MUST_NOT_REMAIN_AUTHORITATIVE = YES
```

## POOL_SHRINK / POOL_GROW / TTL / MAX SEMANTICS

```text
TTL_DECREASE_POLICY =
  After ensure_pool with shorter ttl: idle older than new ttl discarded immediately;
  subsequent take cannot reuse them. Netcup oracle: SYN≥1 after sleep > new ttl.

TTL_INCREASE_POLICY =
  Same deadline model with longer ttl; already-idle may remain reusable longer
  under new policy (idle_since unchanged). Deterministic.

MAX_CONNECTIONS_DECREASE_POLICY =
  Evict excess idle immediately to new max.
  Active may temporarily exceed new max (not in pool).
  NO_NEW_CAPACITY_BEYOND_NEW_LIMIT on put/take path.
  EVENTUAL_POOL_BOUND_AFTER_SHRINK when actives complete and put/discard.

MAX_CONNECTIONS_INCREASE_POLICY =
  New max applied; put may retain up to new capacity (POOL_GROW_AFTER_RELOAD).

POOL_SHRINK_SEMANTICS / POOL_GROW_SEMANTICS = as above.
```

## WAITERS_OR_PENDING_ACQUISITIONS / NEW_CONNECTIONS

```text
NEW_CONNECTIONS = connect() when take misses; uses live CompiledFcgiPool timeouts.
WAITERS_OR_PENDING_ACQUISITIONS =
  CFD FastCGI path has no async waiter queue on FcgiPoolMap;
  miss → connect or fail per execute_get. N/A for queue semantics.
```

## ERROR_SEMANTICS / POISON_SEMANTICS / FAIL_CLOSED_BEHAVIOR

```text
Probe Dirty / gen mismatch / over-cap put / TTL-expired idle → DISCARD (shutdown).
ensure_pool never reports success while leaving stale authoritative TTL/max.
Partial apply forbidden: assign both fields then sweep.
Backend reset during sweep: discard closes local FD; no torn policy.
Shutdown during update: shard drop owns map; no global registry.
```

## HOT_PATH_IMPACT

```text
NEW_MUTEX_OPS_PER_REQUEST = 0
NEW_RWLOCK_OPS_PER_REQUEST = 0
NEW_ATOMIC_OPS_PER_REQUEST = 0
NEW_ALLOCATIONS_PER_REQUEST = 0 (steady; sweep may allocate kept Vec only on change path)
NEW_CLONES_PER_REQUEST = 0
NEW_GLOBAL_HOT_LOCK = NO

Policy refresh cost is O(idle) on ensure_pool when fields change (and always when
called — assign is O(1); sweep O(idle)). Prefer change-gated early-out if equal
to avoid idle walk when unchanged.
```

## Arc / Mutex / RwLock / atomics / allocations / clones / Box/dyn / lifetimes / RAII / newtypes

```text
No new Arc, Mutex, RwLock, atomics, Box<dyn>, typestate, or RAII lease types.
No new lifetimes across threads.
Retain existing IdleConn / PoolIdle structs; extend ensure_pool behavior only.
```

## CLEANUP / SSOT / ROLLBACK

```text
POOL_POLICY_SSOT =
  CURRENT_PUBLISHED_GENERATION.CompiledFcgiPool.{idle_timeout_ms,max_connections}
  applied into PoolIdle on ensure_pool

Forbidden SSOT: first-created PoolIdle fields forever.

ROLLBACK = revert ensure_pool to or_insert_with-only (restores defect); no schema migration.
POOL_MAP_ENTRY_GROWTH_FOR_SAME_POOL_ID = 0_UNBOUNDED (same key; refresh in place).
```

## SECURITY / PERFORMANCE / OBSERVABILITY CONSEQUENCES

```text
SECURITY:
  Closes stale-policy / idle-expiry / max-shrink-bypass classes for same pool_id.
  Does not weaken P6C activation, SG_FCGI_04/05/06, or retry (NotStarted max1).

PERFORMANCE:
  No competitive bench. KEEP_CONN preserved under stable policy.
  Reject reconnect-every-request.

OBSERVABILITY:
  No new per-request metrics required for close.
  Harness SUMMARY: unmeasured claims → NOT_MEASURED (honesty WIP).
```

## PRESERVE

```text
P6C CHOSEN_DESIGN = B_PRE_BEGIN_ALWAYS_PROBE
GENERATION_CHECK_AT_FINAL_ACTIVATION = PASS (unchanged)
FCGI_POST_PROBE_TO_FIRST_WRITE_MICRO_WINDOW = DOCUMENTED_NONBLOCKING_FUTURE_REVIEW
GENERIC_H1_POOL / ADR-021 = DO_NOT_TOUCH
TOKIO_THREADS_IN_CFD = 0
HYPER_ON_CFD_FASTCGI_PATH = 0
CONTROL_PLANE_RPC_PER_REQUEST = 0
```

## IMPLEMENTATION SURFACE

```text
PRIMARY = crates/exyonq-cfd-dataplane/src/fcgi_pool.rs :: ensure_pool (+ tests)
HARNESS = scripts/reality/run-phase6d-fastcgi-resource-concurrency-stress-netcup.sh
  SUMMARY honesty + S8 eviction SYN hard gate
SECOND_PRODUCT_FILE = require WHY_REQUIRED
```

## ACCEPTANCE (repair)

```text
Component: TTL↓/↑, max↓/↑, same pool_id, shrink with active>max eventual, KEEP_CONN
Netcup: S8_evict SYN≥1 after idle_ms=2000 + sleep>2s; S8_long KEEP_CONN preserved
Netcup: S9 pool-bound reload contamination/FD; grow/shrink component proofs
ZERO_FAKE / NO_SMOKE / ANTI_KOBAYASHI (no always-reconnect)
```

## DECISION

Accept **D**: refresh `PoolIdle` policy from live `CompiledFcgiPool` on `ensure_pool` and immediately sweep idle that violate new TTL or exceed new max. Preserve active-completion + put reconsider. No new pooling abstraction.
