# ADR 042 — CFD FastCGI `max_connections` config honesty (fail-closed ≤1)

```text
ADR = 042
TITLE = CFD FastCGI max_connections config honesty (reject values other than 1)
STATUS = ACCEPTED
DATE = 2026-08-31
WIP = V044_PHASE6_FASTCGI_MAX_CONNECTIONS_CONFIG_HONESTY_CLOSE
PARENT_TERMINAL = P6MAX-C (IMMUTABLE)
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
DO_NOT_REUSE = ADR-039 | ADR-040 | ADR-041
ORCHESTRATOR = CONDITIONAL_ACCEPT
VERTEX_PRE_MUTATION = CONDITIONAL_ACCEPT (FAIL_CLOSED_GT1; keep name)
SECURITY_PRE_MUTATION = CONDITIONAL_ACCEPT (RESOURCE_BOUND_PUBLIC_CLAIM_ALLOWED=NO)
RESEARCH_PRE_MUTATION = PASS_WITH_REMEDIATION (rename advisory; REJECT_GT1 accepted)
GRC_PRE_MUTATION = CONDITIONAL_ACCEPT (ADR before mutation)
PRODUCT_MUTATION_AUTHORIZED = YES_AFTER_THIS_ADR
WORDPRESS = NO
FASTCGI_CONCURRENCY_IMPLEMENTATION = NO
```

## Context

P6MAX-C proved on the supported CFD FastCGI path (Netcup amd64, real PHP-FPM):

```text
MAX_CONNECTIONS_SCOPE = PER_SHARD_PER_POOL_ID_IDLE_ONLY_CAP
MAX_CONNECTIONS_GT1_SEMANTICS = INOPERATIVE_CURRENTLY
MAX_GT_1_RUNTIME_EFFECT_POSSIBLE = NO
CONFIG_CONTRACT_STATUS = MISLEADING
```

Under serial blocking `execute_get` per shard, `max_connections` ∈ {1,2,6} produced identical externally observed backend ESTAB and PHP overlap (=1). With KEEP_CONN, idle retention also cannot accumulate beyond one socket per shard map because only one connection is ever in flight.

`config/ir` historically documented FastCGI `max_connections` as a “Physical socket cap (busy + idle)”. That wording is **not** truthful for the CFD project route field `max_conn`, which only gates idle list length at `FcgiPoolMap::put` / `ensure_pool` shrink.

This ADR closes **config honesty**, not the old active+idle capacity oracle. P6MAX-C meaning remains immutable.

## Decision

```text
CHOSEN_CONFIG_STRATEGY = REJECT_GT1
FAIL_CLOSED_GT1 = SELECTED
NEW_CONFIG_NAME = NONE
LEGACY_MAX_CONNECTIONS_POLICY = CFD_PROJECT_FIELD_RETAINED_VALUE_MUST_BE_1
```

### CFD project format (`fcgi|…|max_conn|…`)

| Value | Behavior |
|-------|----------|
| `1` | **Accepted** — sole supported value under current serial CFD FastCGI |
| `0` | **Rejected** — no silent coerce-to-1 |
| `>1` | **Rejected** — no silent no-op concurrent / idle-capacity promise |

Error message must state that CFD FastCGI execution is serial per shard, that values other than `1` do not represent concurrent backend capacity, and that `max_conn=1` is required.

### Field name

Keep the CFD / IR field name `max_connections` / route column `max_conn` for now:

- renaming alone does not fix silent acceptance of meaningless values;
- fail-closed `=1` removes the operator resource-bound illusion;
- a future multi-inflight architecture may introduce an **explicit** concurrent capacity field under a separate ADR (not authorized here).

### Runtime `FcgiPoolMap`

No change to `execute_get` geometry. Idle-cap enforcement code may remain (including unit tests that construct `CompiledFcgiPool` with `max_connections > 1` to prove shrink/discard mechanics). **Supported published configuration** cannot supply those values.

### `config/ir` / `exyonq-mod-fastcgi`

Do **not** apply CFD `max_conn=1` reject to Hyper/`exyonq-mod-fastcgi` TOML `FcgiPoolConfig.max_connections` in this ADR. That path owns a distinct open-socket pool and is out of P6MAX-C evidence scope.

Update IR documentation to **stop claiming** a universal busy+idle capacity for all FastCGI surfaces; distinguish CFD vs module-path contracts.

### Importers

`compat/nginx` currently maps `max_connections: None` for FastCGI pools — no importer drift to repair. Do not map nginx `max_conns` / keepalive onto CFD `max_conn` as concurrent capacity.

## Non-goals

- Async / multi-inflight FastCGI
- Shard / scheduler changes
- WordPress
- Converting P6MAX-C into `MAX_CONNECTIONS_RELOAD = STRONG_CAUSAL_PASS`
- Renaming the wire/IR field in this WIP
- Changing `exyonq-mod-fastcgi` concurrency validation

## Architecture invariants

```text
TOKIO_THREADS_IN_CFD = 0
HYPER_ON_CFD_FASTCGI_PATH = 0
CONTROL_PLANE_RPC_PER_REQUEST = 0
NEW_GLOBAL_HOT_LOCK = NO
GENERIC_H1_POOL_CHANGED = NO
NEW_MUTEX_OPS_PER_REQUEST = 0
NEW_RWLOCK_OPS_PER_REQUEST = 0
NEW_ATOMIC_OPS_PER_REQUEST = 0
NEW_ALLOCATIONS_PER_REQUEST = 0
NEW_CLONES_PER_REQUEST = 0
NEW_SYSCALLS_PER_REQUEST = 0
```

Validation runs at CFD project parse / control compile only.

## Future concurrency

If true multi-inflight CFD FastCGI is later authorized, introduce an explicit concurrent capacity contract (new field or revived semantics under a new ADR). Do **not** silently re-enable `max_conn > 1` without that ADR.

## Consequences

- Existing CFD fixtures/tests/harnesses using `max_conn` ≠ 1 must be updated to `1`.
- Operators cannot configure a misleading concurrent FastCGI socket budget via CFD.
- P6MAX-C historical evidence stays as the proof that `>1` was inoperative before this fail-closed gate.
