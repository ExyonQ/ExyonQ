# ADR 041 — Phase 6 pre-WordPress FastCGI residual honesty (script_suffix + reuse_ok)

```text
ADR = 041
TITLE = Phase 6 pre-WordPress FastCGI residual honesty (script_suffix fail-closed + dead reuse_ok removal)
STATUS = ACCEPTED
DATE = 2026-08-31
WIP = V044_PHASE6_FASTCGI_PRE_WORDPRESS_BLOCKER_REPAIR_AND_ATTRIBUTION_CLOSE
PARENT_AUTHORITY = P6DRC-B
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
DO_NOT_REUSE = ADR-039 | ADR-040 | ADR-021
ORCHESTRATOR = CONDITIONAL_ACCEPT
VERTEX_PRE_MUTATION = CONDITIONAL_ACCEPT (A2-B + A3-remove; A1 no-mutate)
SECURITY_PRE_MUTATION = SECURITY_ACCEPT (MUST_FIX=0; A2 should-fix honesty; A1-A)
GRC_PRE_MUTATION = CONDITIONAL_ACCEPT
WORDPRESS = NO
PRODUCT_MUTATION_AUTHORIZED = YES_AFTER_THIS_ADR
```

## Context

P6DRC-B closed Phase 6D residuals with nonblocking qualifications. Before WordPress, two distinct problem classes remain:

- **Track A** — FastCGI product honesty residuals (this ADR)
- **Track B** — workspace attribution / proxy_sdp hang (out of this ADR; separate evidence track)

## Decisions

### A1 — SG-FCGI-06 peercred

```text
MUTATION = NO
OUTCOME = A1-B CURRENT_SINGLE_TENANT_ADMIN_FS_MODEL_SUFFICIENT_PEERCRED_FUTURE_HARDENING
CLASSIFICATION = VALID_QUALIFICATION_NO_FIX_REQUIRED
```

Keep absolute-path Unix socket trust (`SINGLE_TENANT_ADMIN_FS`). Do **not** implement `SO_PEERCRED` in this WIP. Multi-tenant peercred requires a future trust-model ADR + security admission.

Documented deployment preconditions (operator-owned):

- admin-controlled configuration of absolute Unix socket path
- secure parent directories and socket replacement threat model remain **operator / deployment** responsibilities (product does not invent parent-dir permission checks it does not perform)
- TCP PHP-FPM remains valid; peercred N/A on TCP

### A2 — `script_suffix`

```text
PROBLEM = CONFIG_ACCEPTED_BUT_SEMANTICALLY_IGNORED = YES
  # control project optional field + IR round-trip; SCRIPT_FILENAME ignores field
CLASSIFICATION = REAL_SUPPORTED_HARDENING_SHOULD_FIX_NOW
  # owner §13: silently ignored supported config → MUST_FIX_OR_FAIL_CLOSED
DIRECTION = B_REJECT_UNSUPPORTED_NON_EMPTY
MUTATION = YES
```

**Chosen behavior:**

- empty / omitted `script_suffix` → allowed (no semantic claim)
- non-empty `script_suffix` → **hard fail** at control project parse (and IR load if present)
- do **not** implement suffix routing / SCRIPT_FILENAME append in this WIP
- `script_filename_under_root(document_root, path)` remains sole SCRIPT_FILENAME authority

### A3 — FastCGI `FcgiExecOk.reuse_ok`

```text
PROBLEM = always true on success; unread; #[allow(dead_code)]; put() already done
CLASSIFICATION = DEAD_OR_UNUSED_CODE_CLEANUP
DIRECTION = REMOVE_DEAD_FIELD
MUTATION = YES
CHECKIN_SSOT = FcgiPoolMap::put (re-probe) — unchanged
NOT_ADR021 = YES  # H1 Hybrid F reuse_ok is a different symbol/path
```

Do **not** introduce a second FCGI reuse gate. Do **not** conflate with ADR-021.

## Non-goals

- WordPress / CMS path mapping
- Peercred implementation
- Generic H1 pool changes
- Tokio / Hyper on CFD FastCGI path
- Rewriting P6D-C / P6DR-B / P6DRC-B historical terminals

## Architecture invariants

```text
TOKIO_THREADS_IN_CFD = 0
HYPER_ON_CFD_FASTCGI_PATH = 0
NEW_GLOBAL_HOT_LOCK = NO
GENERIC_H1_POOL_CHANGED = NO
NO_NEW_PATTERN = SUFFICIENT
```

## Hot-path budget

```text
NEW_MUTEX_OPS_PER_REQUEST = 0
NEW_RWLOCK_OPS_PER_REQUEST = 0
NEW_ATOMIC_OPS_PER_REQUEST = 0
NEW_ALLOCATIONS_PER_REQUEST = 0  # reject path only on bad config
NEW_CLONES_PER_REQUEST = 0
NEW_SYSCALLS_PER_REQUEST = 0
```

## Consequences

- Operators who set a non-empty `script_suffix` get an explicit compile/publish error instead of silent no-op
- Observability no longer exposes a lying `reuse_ok` on FastCGI success
- Peercred remains a future multi-tenant gate, not a pre-WordPress blocker under the current contract
