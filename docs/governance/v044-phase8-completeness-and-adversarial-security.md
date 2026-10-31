# V044 — Phase 8 completeness and adversarial security

```text
WIP = V044_PHASE8_COMPLETENESS_AND_ADVERSARIAL_SECURITY
PARENT_TERMINAL = STATICDUAL-B
PRIMARY_PLATFORM = LINUX_AMD64_NETCUP
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
TERMINAL_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
TERMINAL_TREE = ac059750141ee48f141c543cccbd641652bf49fb
AUTHORITATIVE_RUN = 20260901T181000Z
INVALID_RUN = 20260901T175801Z
EVIDENCE = .exyonq-local/evidence/phase8-completeness-adversarial-security/
PRODUCT_MUTATION = NO
PRODUCT_REPAIR_AUTHORIZED = NO
PRODUCT_REPAIR_PERFORMED = NO
HARNESS_MUTATION = YES_EVIDENCE_ONLY
TEST_MUTATION = NO
DOC_MUTATION = YES
PHASE_9_AUTHORIZED = NO
CASE = P8-C
```

Phase 8 is an **attack / completeness** campaign against the current supported CFD HTTP/1.1 three-handler dataplane. It is **not** feature development. It does **not** rewrite STATICDUAL-B, P7R-B, P7STATICIMPL-B, P7STATICADM-B, P7-E, P6G-A, P6F-A, P6EROUTE-A, or CAP067.

**CASE P8-C** means `SUPPORTED_SECURITY_OR_CORRECTNESS_DEFECT_FOUND` and **STOP**. It is **not** a completeness close, security ACCEPT, or shipping admission.

## Admission (pre-evidence)

Before Netcup execution, specialists bound scope and attack plan only (`PRODUCT_MUTATION=NO`):

| Role | Phase | Verdict | Notes |
| --- | --- | --- | --- |
| Orchestrator | pre-evidence | **CONDITIONAL ACCEPT** | Evidence-only WIP; WAF-off P7R topology for WS1–WS4; no product/async FCGI/Phase 9 ([review](3dbc940c-d1d3-4e73-9f80-f452730831ff)) |
| Vertex | pre-evidence brief | **BLOCKED** | Source reconstruction only; `RUNTIME_EVIDENCE_ABSENT` — no safety PASS without Linux campaign ([review](88f5c947-d044-467b-9df7-445ebc8861bf)) |
| Security engineer | pre-evidence plan | **CONDITIONAL ACCEPT** | Attack plan, not close; TOCTOU / FCGI dup / stale-gen still `NOT_MEASURED` ([review](90b01b9e-e1c7-45e7-b62f-1fd9ad462380)) |
| Research | pre-evidence | **COMPLETE** | CFD archaeology + oracle matrix; `PASS_CLAIM=FORBIDDEN_THIS_REVIEW` ([review](7163d516-7bac-4535-8ae0-7fb62a178455)) |

Post-evidence terminal falsification (authoritative run `20260901T181000Z`) is in **Specialists** below. Pre-evidence **CONDITIONAL ACCEPT** does **not** authorize completeness close; post-evidence **REJECT** / **FAIL_PROVEN_DEFECTS** is expected for P8-C.

## Finding (MUST_FIX)

```text
ID = P8-STATIC-FIFO-001
EPISTEMIC = PROVEN
CLASS = PRODUCT
SUPPORTED_REACHABLE = YES
SEVERITY = P1
MUST_FIX = YES
BLOCKS_PHASE8_CLOSE = YES
PRODUCT_REPAIR_AUTHORIZED = NO
```

On Netcup amd64, binary `6d9bc14db019633ecbd3519ea8cb8e78ce11a76abaedb0d039f95bb9ad8c413e` (`BINARY_IDENTITY=PASS`, same as P7R-B):

1. Document root contained a FIFO `fifo.pipe`.
2. `GET /static/fifo.pipe` entered Linux `openat2` (`syscall 437`) with `O_RDONLY|O_CLOEXEC` and **no** `O_NONBLOCK`.
3. Shard task stack: `wait_for_partner` → `fifo_open` → `do_sys_openat2`.
4. Subsequent `GET /static/hello.txt` timed out (`http_code=000`, 0 bytes).
5. Killing the client did **not** unblock the shard.

`REGULAR_FILES_ONLY` is applied in `classify_opened` **after** open returns. The Linux production path never reaches that check for a FIFO with no writer. The canonicalize fallback **does** set `O_NONBLOCK`; that fallback is not the Linux `openat2` path.

STATICDUAL-B historically recorded FIFO as “harness dataplane restart (not a product repair)”. That **historical sentence is not rewritten**. Phase 8 kernel stacks **reclassify going forward**: this is a product shard-liveness defect.

Evidence: `20260901T181000Z/ORACLE/fifo_hang/kernel_stacks.txt`.

Source: `crates/exyonq-cfd-dataplane/src/static_serve.rs` `openat2_confined`.

## Invalid / aborted runs

| Run | Class | Use as product truth |
| --- | --- | --- |
| `20260901T175801Z` | `INVALID_HARNESS` | **NO** — absolute symlink TOCTOU; `strace -p` + SIGINT interrupted the dataplane; live `RESULTS.env` classifiers contradict raw JSON |
| `20260901T181000Z` | Authoritative attack; **aborted** after FIFO hang | Pre-FIFO slices usable; post-FIFO stages **NOT_MEASURED** |

Remote wrapper exit **143** (`SIGTERM`) on run `20260901T181000Z` is **operator stop after FIFO kernel-stack capture**, not a harness self-classification. Abort class is `PROVEN_PRODUCT_FIFO_HANG` (`LEDGER/INVALID_AND_ABORTED.txt`), not `INVALID_HARNESS`.

## Pre-FIFO measurements (181000Z, independent oracles)

Usable because they completed **before** the FIFO GET:

| Area | Oracle | Result |
| --- | --- | --- |
| HTTP framing | `HTTP/raw/framing.json` | Fail-closed 400 on duplicate CL, CL+TE, TE-only, malformed TE, missing Host (H1.1), malformed chunked, extra bytes after CL0. `unexpected_success=[]`. HTTP/1.0 no-Host = 200 (in-parser policy, not H1.1 Host confusion). |
| Request desync | raw-socket client + body markers | `MEASURED_0` **on constructed wires only**. No Static body bleed. Pipeline → `400 pipelining unsupported`. Not a universal “no smuggling” claim. |
| Host routing | `HTTP/raw/host.json` | Duplicate/missing Host → 400. `HOST_ROUTE_CONFUSION=0`. |
| Path ownership | independent reclass of `path.json` | Outside-secret bytes 0; no Proxy/FastCGI body on `/static`. `HANDLER_OWNERSHIP_CONFUSION=0`. |
| Keepalive mix | independent reclass of `keepalive.json` | Cross-handler markers as expected. `MEASURED_0`. Live harness classifier crashed (`read_text` on a file object) — do not use truncated `RESULTS.env` for this key. |
| One handler | one_static / one_proxy / one_fcgi | `MULTI_HANDLER_EXECUTION=MEASURED_0` |
| TOCTOU | relative `safe-target.txt` vs `../outside-secret`, N=4000 | SAFE=271 BLOCKED=3729 OUTSIDE=0. In-root relative symlink **PASS**. Out-of-root **BLOCKED**. |
| openat2 flags | sidecar strace (never `-p` on live DP) | `RESOLVE_BENEATH\|RESOLVE_NO_MAGICLINKS` + `O_RDONLY\|O_CLOEXEC` |
| sendfile EAGAIN | sidecar strace | EAGAIN then retry at offset `984960`. Live 2 MiB hash incomplete (`SLOW_LEN=150400`). |
| Client disconnect | before FIFO | next GET 200; FD delta 0 (narrow) |

Campaign **stopped** after MUST_FIX. Not repaired.

## Completeness matrix

| AREA | CLAIM | PRE_PHASE8 | PHASE8_ORACLE | PHASE8_RESULT | NEW_STATUS | OPEN_DEFECT? | FUTURE_REVIEW? |
| --- | --- | --- | --- | --- | --- | --- | --- |
| HTTP framing | fail-closed parse | source + P7 | raw sockets | FAIL_CLOSED on measured wires | STRONGLY_SUPPORTED (measured set) | NO on measured set | YES broader matrix |
| request desync | no FE/BE disagreement | not adversarial | constructed wires | MEASURED_0 constructed | BOUNDED | NO proven desync | YES |
| host routing | Cap033 | P7R | host.json | 0 confusion | STRONGLY_SUPPORTED | NO | NO unless contract change |
| path ownership | one handler | P7R | path.json | 0 | STRONGLY_SUPPORTED | NO | encoded FCGI `..` residual |
| Static containment | no outside bytes | STATICDUAL | TOCTOU + path | 0 outside | STRONGLY_SUPPORTED this campaign | NO escape proven | YES TOCTOU residual |
| Static TOCTOU | race escape | BOUNDED_RESIDUAL | relative 4000 | 0 outside, not closed | NOT_REPRODUCED_RESIDUAL_PRESERVED | NO MUST_FIX | YES shipping admission |
| Static pread fallback | runtime | MISSING | sendfile succeeded | NOT_MEASURED_WITH_REASON | PRESERVED_NONBLOCKING | NO | YES if sendfile unavailable |
| Static EAGAIN | park, no dup/loss | source only | strace EAGAIN; live hash incomplete | CLOSED_BOUNDED | NO | YES full-hash |
| Static partial send | offset progress | source only | multi sendfile offsets | CLOSED_BOUNDED | NO | YES full-hash |
| Static disconnect | no leak | P7 | pre-FIFO FD Δ=0 | STRONGLY_SUPPORTED narrow | NO | YES after FIFO fix |
| Proxy target control | client cannot steer | P7R | aborted after FIFO | NOT_MEASURED | PRESERVED | UNKNOWN | YES |
| Proxy duplicate | one side effect | P7R MEASURED_0 | aborted | NOT_MEASURED | PRESERVED | UNKNOWN | YES |
| FastCGI mapping | server SCRIPT_FILENAME | source | keepalive FCGI_OK only | SOURCE_PROOF; runtime mapping aborted | PRESERVED | UNKNOWN | YES |
| FastCGI duplicate | durable oracle | NOT_MEASURED | aborted | NOT_MEASURED | PRESERVED | UNKNOWN | YES |
| FastCGI retry | NotStarted max 1 | source | aborted | NOT_MEASURED | PRESERVED | UNKNOWN | YES |
| generation stale crossover | no mixed ownership | NOT_MEASURED | aborted | NOT_MEASURED | PRESERVED | UNKNOWN | YES |
| generation malformed | reject, keep previous | P7 | aborted | NOT_MEASURED | PRESERVED | UNKNOWN | YES |
| generation atomicity | no mixed gen | P7 | aborted | NOT_MEASURED | PRESERVED | UNKNOWN | YES |
| cross-handler keepalive | no contamination | P7R | keepalive.json | MEASURED_0 pre-FIFO | STRONGLY_SUPPORTED | NO | YES after FIFO fix |
| resource lifetime | no staircase | P7 | aborted | NOT_MEASURED after FIFO; shard stuck | DEFECT FIFO | YES P8-STATIC-FIFO-001 | YES |
| sensitive-data observability | no secret logs | not Phase 8 | aborted | NOT_MEASURED | PRESERVED | UNKNOWN | YES |
| **Static special files** | REGULAR_FILES_ONLY, no hang | STATICDUAL “harness restart” | kernel stack + hello timeout | **HANG** | **PROMOTED_TO_MUST_FIX** | **YES P1** | Repair WIP |

## Residuals R1–R9 (exactly one token each)

See [v044-phase8-residual-ledger.md](v044-phase8-residual-ledger.md).

FIFO is **not** R9. It is a new MUST_FIX beside the entry ledger.

## Specialists

| Role | Verdict |
| --- | --- |
| Vertex | **REJECT** ([review](73d98c1d-9747-40ad-8dc7-d58449fa656f)) |
| Security | **REJECT** ([review](ca0c62c5-f02c-4174-abbd-78b61b43a2cf)) |
| Research | **CONDITIONAL** ([review](ad181db2-f842-486a-b6ed-7d6c948f4d03)) |
| GRC | **CONDITIONAL** (honesty of residuals; Phase 8 close blocked) ([review](04545eed-6196-4563-9cbc-ab36f9cac423)) |
| Logic | **FAIL_PROVEN_DEFECTS**; `PHASE8_FALSIFIED=YES` (completeness/security **close**); FIFO hang **confirmed** ([review](c30ba857-58af-477f-9b83-f2e40b4ae936)) |

`P8-C` as owner CASE (defect found, STOP) is the honest terminal. Completeness close (P8-A/B) would be inflation.

## WAF

```text
WAF_PROFILE = NOT_PUBLISHED_MATCHES_P7R_SUPPORTED_PATH
```

No WAF Phase 1 reopen. `cfd-publish-routes` did not publish a WAF projection on this topology.

## Integrity

```text
ZERO_FAKE = PASS
NO_SMOKE = PASS
ANTI_KOBAYASHI = PASS
USES_REAL_DATA = YES
USES_SYNTHETIC_FIXTURES = YES_LOCAL_DOCROOT_NOT_PRODUCT_RESULT
CHANGES_PRODUCT_SEMANTICS = NO
```

Workspace `cargo test --workspace` was **not** completed on Netcup this run (abort). Ambient `exyonq-metrics` test is **not** attributed to Phase 8 without a current re-run.

## Shipping admission

```text
REVIEW_BEFORE_FINAL_SHIPPING_ADMISSION = YES
SYMLINK_TOCTOU_FINAL_PHASE8_STATUS = NOT_REPRODUCED_RESIDUAL_PRESERVED
P8_STATIC_FIFO_001 = MUST_FIX_BEFORE_STATIC_SPECIAL_FILE_CONTRACT_CLOSE
IS_PHASE9_PREREQUISITE_POSTURE_READY = NO
```

**Later event:** owner authorized [v044-static-openat2-nonblock-special-file-repair.md](v044-static-openat2-nonblock-special-file-repair.md) — **P8R-B** closes `P8-STATIC-FIFO-001` on dual Linux FIFO liveness; P8-C remains historical valid defect terminal.

Repair of Linux Static `openat2` so FIFO/unix/special files fail closed **without** blocking the shard (`O_NONBLOCK` and/or type-check before partner-wait), then Linux E2E that a subsequent Static GET still completes. Then resume remaining Phase 8 oracles (FastCGI durable dup, generation crossover, proxy remaining stages).

```text
NEXT_WIP_AUTHORIZED = NO
PHASE_9_AUTHORIZED = NO
FASTCGI_ASYNC_AUTHORIZED = NO
CANONICAL_MERGE = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION = NO
```
