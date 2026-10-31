# V044 — Static openat2 nonblock special-file repair (architecture admission)

```text
WIP = V044_STATIC_OPENAT2_NONBLOCK_SPECIAL_FILE_REPAIR
PARENT_TERMINAL = P8-C
PRIMARY_DEFECT = P8-STATIC-FIFO-001
PRODUCT_REPAIR_AUTHORIZED = YES_FOR_P8_STATIC_FIFO_001_ONLY
```

## Problem

Linux primary path `openat2_confined` used `O_RDONLY|O_CLOEXEC` without `O_NONBLOCK`. FIFO open blocked in `wait_for_partner` before `classify_opened` (`REGULAR_FILES_ONLY`). Canonicalize fallback already used `O_NONBLOCK`.

## Alternatives evaluated

| Alt | Summary | Verdict |
| --- | --- | --- |
| **A** | `O_NONBLOCK` on `openat2` + post-open `fstat` via `classify_opened` | **SELECTED** |
| **B** | `O_PATH` / probe then second open | **REJECT** — pathname reopen TOCTOU unless `(dev,ino)` binding; extra syscall |
| **C** | Pre-open `fstatat`, thread offload, `OPENAT2_REGULAR` | **REJECT** — TOCTOU, shard model, kernel 7.2+ only |

## Selected architecture

```text
SELECTED_ARCHITECTURE = Alternative A
openat2(O_RDONLY | O_CLOEXEC | O_NONBLOCK, RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS)
→ classify_opened (fstat on same FD)
→ S_IFREG only → StaticSend; else Forbidden (403)
```

```text
OBJECT_IDENTITY_GUARANTEE = OPEN_FD → fstat(metadata) on same FD; no pathname second-open
TOCTOU_MODEL = unchanged BOUNDED_RESIDUAL; repair does not close TOCTOU
SPECIAL_FILE_SIDE_EFFECT_MODEL = O_NONBLOCK prevents FIFO partner-wait; non-regular → Forbidden
EXPECTED_HOT_PATH_DELTA = +0 syscalls; +O_NONBLOCK flag bit on primary openat2
CLIENT_VISIBLE_FAILURE_CONTRACT = FIFO/socket/special → 403 Forbidden; shard must stay live
SUPPORTED_KERNEL_BASELINE = Linux 5.6+ (openat2); Netcup amd64 + Oracle arm64
ROLLBACK_CRITERIA = FIFO shard hang reproduced; regular-file hash mismatch; TOCTOU outside bytes > 0
```

## Rejected alternatives (detail)

**B:** Second pathname open after O_PATH probe reintroduces check-before-use unless `(st_dev,st_ino,mode)` matched — not needed when A makes first open nonblocking.

**C:** Pre-open stat is not the admitted `FILE_IDENTITY_ORACLE`; thread offload violates sync shard model.

## Invariants preserved

```text
TOKIO_THREADS_IN_CFD = 0
REGULAR_FILES_ONLY = REQUIRED (on opened FD)
DOCROOT_CONTAINMENT = REQUIRED (RESOLVE_BENEATH unchanged)
PATHNAME_REOPEN_AFTER_VALIDATION = NO
```

Specialists: Vertex **CONDITIONAL_ACCEPT**, Security **REJECT current / repair via A**, Research **STRONGLY_SUPPORTED O_NONBLOCK hypothesis**.
