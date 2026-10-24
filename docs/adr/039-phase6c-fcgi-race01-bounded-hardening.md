# ADR 039 — Phase 6C FCGI-RACE-01 bounded hardening

```text
ADR = 039
TITLE = Phase 6C FCGI-RACE-01 checkout→BEGIN final activation validation
STATUS = ACCEPTED_PRE_MUTATION_COMPLETE
DATE = 2026-08-30
WIP = V044_PHASE6C_FCGI_RACE01_BOUNDED_HARDENING
PARENT = V044_PHASE6B_FASTCGI_MVP_TERMINAL_SEAL (P6BSEAL-B)
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
PRODUCT_MUTATION_AUTHORIZED = YES_AFTER_VERTEX_SECURITY_GRC_PASS
VERTEX_ARCHITECT_REVIEW = PASS
SECURITY_ENGINEER_PRE_MUTATION_REVIEW = PASS
GRC_PRE_MUTATION_REVIEW = PASS
ORCHESTRATOR_ADMISSION = CONDITIONAL_ACCEPT
PRODUCT_MUTATION_STARTED_UTC = 2026-08-30T16:11:22Z
P6B_PRE_MUTATION_ARCHITECTURE_REVIEW = FAIL   # historical; do not rewrite
P6C_PRE_MUTATION_ARCHITECTURE_REVIEW = PASS
```

## PROBLEM_DEFINITION

After `FcgiPoolMap::take()` returns a probed-clean idle FastCGI socket, `fcgi_exec::execute_get` may apply timeouts and call `exchange_once` (which writes `FCGI_BEGIN_REQUEST`) without a **final activation validation** at the pooled→active boundary. Between checkout probe (T1) and BEGIN write (T4), the peer may EOF, become readable with unexpected bytes, error, or become generation-stale. That TOCTOU is FCGI-RACE-01 (P6B-P3-03).

## CURRENT_STATE

```text
IDLE_POOLED --take(probe+gen+ttl)--> CANDIDATE --apply_timeouts--> exchange_once(BEGIN write)
NO final pre-BEGIN probe
NO generation re-check at activation
FcgiStream = std TcpStream|UnixStream (blocking I/O on shard thread)
NO mio Interest::READABLE registration for FastCGI idle sockets
H1 UpstreamPool (ADR-021) uses dual probe + idle READABLE — DIFFERENT subsystem
```

Source: `fcgi_pool.rs::take` / `probe`; `fcgi_exec.rs::execute_get` lines ~306–333.

## TARGET_STATE

```text
IDLE_POOLED --take(...)--> CANDIDATE --final_activation_validation--> ACTIVE(BEGIN)
FORBIDDEN: CANDIDATE --arbitrary_gap--> ACTIVE without final validation
UNKNOWN|EOF|READABLE|ERROR|GEN_MISMATCH|AMBIGUOUS = DISCARD (+ fresh connect if NotStarted)
```

## ALTERNATIVES

| ID | Design | Verdict |
|----|--------|---------|
| A | Checkout probe only (status quo) | **REJECT** — leaves FCGI-RACE-01 open |
| **B** | Immediate second nonblocking probe + generation re-check immediately before BEGIN | **ACCEPT** — smallest extension of existing `probe()` |
| C | Hold idle READABLE (mio) until ACTIVE | **REJECT for P6C** — introduces mio registration for FCGI std streams; new pattern + FD/token ownership surface; not required if B closes admitted race |
| D | RAII FastcgiLease with activate() | **REJECT** — new abstraction; B can be a method on `FcgiPoolMap` / free fn |
| E | Explicit typestate PooledValidated→Activating→Active | **REJECT** — type noise without extra safety beyond B for sync path |
| F | Always discard / never reuse | **REJECT** — undoes proven causal KEEP_CONN (1000:1); migration-value regression |
| G | Hybrid idle READABLE + pre-BEGIN + fail-closed | **REJECT for P6C** — over-builds; prefer B; G remains future if B residual proven insufficient |
| H | Other | None smaller than B found |

## NO_NEW_PATTERN_ALTERNATIVE

```text
NO_NEW_PATTERN_ALTERNATIVE = SUFFICIENT
CAN_THE_RACE_BE_CLOSED_BY_EXTENDING_EXISTING_FASTCGI_POOL_AND_CONNECTION_STATE
  WITHOUT_INTRODUCING_A_NEW_LIFECYCLE_ABSTRACTION = YES
```

Evidence: `FcgiPoolMap::probe` already implements nonblocking one-byte dirty detection. Calling it again (and re-validating `generation`) immediately before `exchange_once` extends existing pool state with **zero** new Arc/Mutex/mio token types. Hold-READABLE would be a new pattern relative to current FCGI sync pool.

## OWNERSHIP

| Object | Owner | Notes |
|--------|-------|-------|
| `FcgiPoolMap` | Shard thread only (`&mut` in `ShardCtx`) | No cross-thread share |
| Idle `FcgiStream` | `PoolIdle.idle` Vec until `take` moves out | Exclusive |
| Candidate stream | `execute_get` local until put/discard | Exclusive |
| ACTIVE exchange | `exchange_once` borrows `&mut FcgiStream` | Exclusive |
| Generation id | Immutable gen projection + shard local_gen_id | Compared at take + final activation |

## MUTABILITY

```text
FcgiPoolMap: &mut on shard only
FcgiStream: owned move; no Arc<Mutex<TcpStream>>
probe: &mut FcgiStream (temporary nonblocking)
No shared mutable idle map across shards
```

## CONCURRENCY

```text
Shard-local pool — one thread owns map
Concurrent requests on same shard are sequential in event loop for one connection;
  different client conns may interleave but each execute_get holds exclusive stream
No simultaneous lease of same idle entry (Vec pop)
NEW_GLOBAL_HOT_LOCK = FORBIDDEN
```

## LIFECYCLE_STATE_MACHINE

```text
NEW_CONNECT → ACTIVE_EXCHANGE → (success+clean) → IDLE_POOLED
IDLE_POOLED → CANDIDATE (take: ttl/gen/probe)
CANDIDATE → ACTIVE_EXCHANGE (final_activation: gen+probe) OR DISCARD
ACTIVE_EXCHANGE → IDLE_POOLED (put: probe) OR DISCARD (any error/poison)
IDLE_POOLED → DISCARD (flush_generation / ttl / dirty put)

FORBIDDEN:
  IDLE_POOLED → ACTIVE_EXCHANGE without CANDIDATE final_activation
  CANDIDATE → IDLE_POOLED (must discard or activate; no re-idle without exchange)
  POISONED → IDLE_POOLED
```

## ERROR_SEMANTICS / POISON_SEMANTICS

```text
Probe Clean (WouldBlock) = continue
Probe Dirty (Ok(0)|Ok(n)|Err(non-WouldBlock)) = DISCARD
apply_timeouts fail = DISCARD + Connect error class
exchange TrailingData/Timeout/Protocol = DISCARD; no put
CGI parse fail = DISCARD
Final activation Dirty = DISCARD; if pool_hit, fresh connect (still NotStarted)
Partial BEGIN after CommitStage::BeginCommitted = NO_RETRY
```

## INVARIANTS

```text
TOKIO_IN_CFD = 0
HYPER_IN_CFD = 0
CONTROL_PLANE_RPC_PER_REQUEST = 0
CROSS_PROCESS_RPC_PER_REQUEST = 0
DUAL_EXECUTION_OWNER = NO
TRANSPARENT_FALLBACK = NO
NEW_GLOBAL_HOT_LOCK = NO
GENERIC_H1_POOL_CHANGED = NO
FASTCGI_MAX_RETRY = 1
RETRY_ONLY_IF = NotStarted
SG-FCGI-04/05 remain CLOSED
SG-FCGI-06 remains QUALIFIED_NONBLOCKING (no peercred expansion)
```

## HOT_PATH_IMPACT

```text
On pool hit: +1 nonblocking read probe + generation compare before BEGIN
On pool miss (fresh connect): optional skip of redundant probe (socket new) OR still probe (cheap)
Expected: sub-microsecond probe; no alloc
```

## ARC_USE / MUTEX_USE / RWLOCK_USE / ATOMICS_USE

```text
ARC_USE = 0 new
MUTEX_USE = 0 new
RWLOCK_USE = 0 new
ATOMICS_USE = existing obs counters only (unchanged semantics)
```

## ALLOCATIONS / CLONES / BOX_DYN / LIFETIMES / RAII / NEWTYPE_TYPESTATE

```text
NEW_PER_REQUEST_ALLOCATIONS = 0
NEW_CLONES = 0
BOX_DYN = 0
LIFETIMES = existing &mut stream
RAII = discard on Dirty via existing FcgiPoolMap::discard (shutdown)
NEWTYPE_TYPESTATE = NOT INTRODUCED (NO_NEW_PATTERN)
```

## CLEANUP

```text
Dirty candidate: discard immediately
Generation flush: existing flush_generation unchanged
Shutdown: shard drop discards map
No readiness tokens to free (sync probe only)
```

## SSOT

```text
Probe semantics SSOT = FcgiPoolMap::probe
Activation boundary SSOT = new activate_pooled_for_begin (or equivalent) called only from execute_get before exchange_once
Wire commit/retry SSOT = exyonq-fastcgi-wire CommitStage (unchanged)
H1 UpstreamPool remains separate SSOT (ADR-021) — do not merge
```

## ROLLBACK

```text
Kill-switch: revert activate call; restore take→exchange direct path (re-opens FCGI-RACE-01)
Binary rollback: prior release binary without activation probe
No config flag required for MVP; code revert is rollback
```

## SECURITY_CONSEQUENCES

```text
Closes admitted TOCTOU for unexpected peer state between checkout and BEGIN
Residual post-probe→first-write window: analyzed below; fail-closed on contamination
Does not claim multi-tenant Unix peercred (SG-FCGI-06 unchanged)
Does not weaken trailing-byte / CL contracts
```

## PERFORMANCE_CONSEQUENCES

```text
Not an optimization WIP
One extra nonblocking read on pool-hit path
Causal KEEP_CONN must remain effective (accepts ≪ N) after change
Sanity-only ExyonQ metrics optional; rivals forbidden
```

## OBSERVABILITY_CONSEQUENCES

```text
May increment fcgi_pool_discard on activation Dirty
Optional: count activation_discard (only if zero-cost atomic already patterned) — prefer reuse fcgi_pool_discard
```

## GENERATION_CONSEQUENCES

```text
GENERATION_CHECK_AT_FINAL_ACTIVATION = PASS
BINDING_MECHANISM =
  take() already discards idle entries with c.generation != generation
  execute_get is synchronous on the shard thread; flush_generation runs only at
  event-loop top — gen cannot bump mid-execute_get while a candidate is held
  Therefore an explicit second generation compare at activation is OPTIONAL SSOT
  documentation, not a required additional atomic. Implementation MAY pass the
  same `generation` argument through activate for assert/documentation only.
FORBIDDEN_CLAIM = "activation re-reads live gen from a shared atomic mid-call"
  # no such shared gen atomic on this path today
```

## FAIL_CLOSED_BEHAVIOR

```text
UNKNOWN = DISCARD
EOF = DISCARD
PENDING_UNEXPECTED_BYTES = DISCARD
SOCKET_ERROR = DISCARD
EXPIRED = DISCARD (ttl at take)
GENERATION_MISMATCH = DISCARD
POISONED = DISCARD
AMBIGUOUS_READINESS = DISCARD
```

## PRE-BEGIN VALIDATION SEMANTICS (mio/std sockets)

Current FCGI path uses `std::net` with temporary `set_nonblocking(true)` + `read(1)`:

| Result | Meaning | Action |
|--------|---------|--------|
| `Err(WouldBlock)` | No pending application data | Clean → ACTIVE |
| `Ok(0)` | EOF | Dirty → DISCARD |
| `Ok(n>0)` | Unexpected byte(s) **consumed** | Dirty → DISCARD (byte not replayed; never fed to next response parser as owned response — connection discarded) |
| `Err(other)` | Socket error | Dirty → DISCARD |

No HUP via this probe API; EOF appears as Ok(0) or subsequent write/read fail.
False-positive WouldBlock under high load: acceptable; residual post-probe handled by protocol fail-closed.
Readiness event registration: **N/A** (no mio token for FCGI idle).

### POST_ACTIVATION_TIMEOUT_RESTORE (material — vertex blocker)

```text
KNOWN_SIDE_EFFECT = FcgiPoolMap::probe currently sets read_timeout = Some(1ms)
  and does NOT restore the prior configured read timeout.
TODAY_MASK = apply_timeouts runs AFTER take's probe, restoring pool read/write timeouts
  before exchange_once.
P6C_ORDER_REQUIRED =
  1. take / connect
  2. activate_before_begin = probe (may leave 1ms read timeout)
  3. apply_timeouts(stream, pool)   # MANDATORY after every activation probe
  4. exchange_once
RETRY_PATH_SAME_ORDER =
  discard → connect → activate probe → apply_timeouts → exchange_once
ALTERNATE_ALLOWED =
  change probe to save/restore prior read timeout (still require apply_timeouts
  after activate for SSOT with pool config); OR stop mutating timeouts inside probe.
FORBIDDEN =
  activate probe AFTER apply_timeouts without a subsequent apply_timeouts /
  restore — that would leave exchange on a 1ms read timeout (correctness/perf bug).
```

## HOLD-READABLE DESIGN ANALYSIS

```text
SELECTED = NOT_USED_IN_P6C
REASON = FCGI pool has no idle mio registration today; adding it is alternative C/G
NO_DOUBLE_OWNERSHIP = N/A (not introducing tokens)
NO_STALE_EVENT_ACTIVATION = N/A
NO_FD_TOKEN_ALIASING = N/A
```

If a future tranche selects C/G, a new ADR is required for token ownership.

## POST_PROBE_RESIDUAL_RACE

```text
CAN_BACKEND_LEGITIMATELY_SEND_FASTCGI_RESPONSE_BYTES_BEFORE_RECEIVING_A_NEW_BEGIN = NO
  # FastCGI responder role: server waits for BEGIN/PARAMS/STDIN before STDOUT/END
IF_UNEXPECTED_BYTES_ARRIVE_AFTER_FINAL_PROBE_BUT_BEFORE_WRITE:
  BEGIN write may still succeed; subsequent read interprets peer bytes as response
  → CGI/protocol parse fail OR TrailingData → DISCARD → no put → no cross-request contamination into next pooled use
  Contaminated response is not returned as success to client if parse fails
PARTIAL write of BEGIN: CommitStage advances per wire crate; retry only if still NotStarted
```

## CHOSEN_DESIGN

```text
CHOSEN_DESIGN = B_PRE_BEGIN_ALWAYS_PROBE
GENERATION_AT_ACTIVATION = PASS_VIA_TAKE_PLUS_SYNC_SHARD_NON_PREEMPTION
  # explicit second gen compare OPTIONAL; not required for correctness on this path
```

### Binding implementation order (minimal)

```text
1. take(pool, generation) OR connect(pool)
2. loop_activate:
     if FcgiPoolMap::probe(&mut stream) != Clean:   # activate_before_begin == probe
         discard(stream)
         if already_retried_connect_for_dirty: return Connect/error
         stream = connect(pool)?; pool_hit = false
         continue loop_activate   # re-probe fresh socket (SSOT: always probe)
     break
3. apply_timeouts(stream, pool)   # MANDATORY after activate probe (timeout restore)
4. exchange_with_total_budget / exchange_once
5. On NotStarted-safe retry:
     discard → connect → probe until Clean → apply_timeouts → exchange again
     MAX one such retry; BeginCommitted ⇒ NO_RETRY
```

```text
activate_before_begin = thin alias or direct call to FcgiPoolMap::probe
ALWAYS_PROBE_BEFORE_EVERY_EXCHANGE_ENTRY = YES
  # pool hit, fresh connect, and NotStarted-retry reconnect — security constraint
POOL_HIT_ONLY_PROBE = FORBIDDEN
NEWTYPE / RAII lease / mio hold-READABLE = NOT INTRODUCED
```

## REJECTED_ALTERNATIVES

A, C, D, E, F, G (for this WIP), H none.

## P6B_P3_04_CAUSAL_TO_RACE01

```text
P6B_P3_04_CAUSAL_TO_RACE01 = NO
script_suffix unused = config inert; not checkout→BEGIN
reuse_ok always true = observability lie after put; not activation race
CARRY_FORWARD unchanged
```

## P6B_P3_02

```text
CARRY_FORWARD = SG_FCGI_06 QUALIFIED_NONBLOCKING
NO peercred work in P6C
```

## GENERIC_H1_POOL_CHANGED

```text
GENERIC_H1_POOL_CHANGED = NO_REQUIRED
```

## P6C_BODY_SCOPE_CHANGED

```text
P6C_BODY_SCOPE_CHANGED = NO
```

## ACCEPTANCE BEFORE PRODUCT MUTATION

```text
VERTEX_ARCHITECT_REVIEW = REQUIRED_PASS
SECURITY_ENGINEER_PRE_MUTATION_REVIEW = REQUIRED_PASS
GRC_PRE_MUTATION_REVIEW = REQUIRED_PASS
ORCHESTRATOR_ADMISSION = REQUIRED
```

## Decision (binding once reviews PASS)

**Accept B** — pre-BEGIN nonblocking probe (reuse `FcgiPoolMap::probe`) on every path into `exchange_once`, with discard+fresh-connect when Dirty and still NotStarted; preserve retry max 1; no mio hold-READABLE; no H1 pool edits; no FastCGI feature expansion.
