# V044 Phase 8 — residual ledger

```text
WIP = V044_PHASE8_COMPLETENESS_AND_ADVERSARIAL_SECURITY
PARENT = STATICDUAL-B
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
AUTHORITATIVE_RUN = 20260901T181000Z
INVALID_RUN = 20260901T175801Z
NO_RESIDUAL_MAY_DISAPPEAR = YES
```

Each entry residual ends as **exactly one** token.

| ID | origin | claim | current evidence | current strength | adversarial test | oracle | result | classification | closure status |
|----|--------|-------|------------------|------------------|------------------|--------|--------|----------------|----------------|
| R1 | P7STATICIMPL-B / STATICDUAL-B | Symlink TOCTOU cannot serve outside-root bytes | Relative race N=4000 SAFE=271 BLOCKED=3729 OUTSIDE=0; in-root relative symlink PASS | NOT_REPRODUCED | High-iteration `ln -sfn` safe vs `../outside-secret` + GET `/static/race-link` | Response bytes vs `outside-secret-*-UNIQUE` | 0 outside | PRODUCT residual, not MUST_FIX | **PRESERVED_NONBLOCKING** |
| R2 | P7STATICIMPL-B | pread fallback runtime | Linux sendfile(file→TCP) succeeds on regular files | NOT_MEASURED_WITH_REASON | Natural EINVAL/ENOSYS not forced | sendfile strace success | fallback not entered | harness did not add a product test switch | **PRESERVED_NONBLOCKING** |
| R3 | P7STATICIMPL-B | EAGAIN / StaticSend park | Sidecar strace `sendfile = -1 EAGAIN` then retry; live 2MiB SHA incomplete len=150400 | BOUNDED | Slow reader SO_RCVBUF | strace + SHA | EAGAIN observed; full completion not hashed | PRODUCT I/O residual | **CLOSED_BOUNDED** |
| R4 | P7STATICIMPL-B | partial sendfile offset | SENDFILE_CALLS=7 PARTIAL_MULTI_CALL=1 offsets 0→984960→1313280; full SHA not matched | BOUNDED | Slow reader + strace | strace offsets | progression observed | PRODUCT I/O residual | **CLOSED_BOUNDED** |
| R5 | P7R-B | Stale generation route crossover | Campaign aborted before publish hammer | NOT_MEASURED | G1 static /switch vs G2 proxy | independent markers | not run | — | **PRESERVED_NONBLOCKING** |
| R6 | P7R-B | FastCGI duplicate execution | Durable PHP side file created; abort before token counts | NOT_MEASURED | Unique tokens + disconnect | `fcgi.side` | not run on live process after FIFO | — | **PRESERVED_NONBLOCKING** |
| R7 | P7R-B | Slow FastCGI isolation | Architectural: sync shard, no async claim | NOT_REMEASURED | slow PHP vs other handlers | — | not remeasured 181000Z | architecture still qualified | **PRESERVED_NONBLOCKING** |
| R8 | P7R-B | Three-handler HTTP runtime ARM64 | This WIP did not produce Oracle three-handler evidence | NOT_PROVEN | — | — | not run | do not upgrade P7R scope | **PRESERVED_NONBLOCKING** |
| R9 | STATICDUAL logic | Harness/oracle hygiene | 175801Z invalid; 181000Z live path/keepalive classifier crash; hardcoded proxy dup stamp in harness (not executed this run) | HARNESS | — | independent JSON reclass | hygiene defects; **not** the FIFO product bug | HARNESS | **PRESERVED_NONBLOCKING** |

## New finding (not an entry residual)

| ID | origin | claim | evidence | strength | classification | closure |
|----|--------|-------|----------|----------|----------------|---------|
| P8-STATIC-FIFO-001 | Phase 8 special-file attack | REGULAR_FILES_ONLY, no hang | Kernel `wait_for_partner`/`fifo_open`/`openat2`; hello.txt timeout | PROVEN | PRODUCT SUPPORTED_REACHABLE P1 | **CLOSED_BY_P8R-B** (see [v044-static-openat2-nonblock-special-file-repair.md](v044-static-openat2-nonblock-special-file-repair.md)) |

`INVALIDATED_AS_NOT_REAL_RESIDUAL` = none.

`CLOSED_STRONG` = none.

TOCTOU:

```text
SYMLINK_TOCTOU_FINAL_PHASE8_STATUS = NOT_REPRODUCED_RESIDUAL_PRESERVED
REVIEW_BEFORE_FINAL_SHIPPING_ADMISSION = YES
```

Do not write `TOCTOU_CLOSED`, `RACE_FREE`, or `IMPOSSIBLE`.
