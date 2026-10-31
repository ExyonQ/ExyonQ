# V044 — Static EAGAIN / partial sendfile / TOCTOU reoracle (P8R-BR)

```text
WIP = V044_STATIC_EAGAIN_PARTIAL_SENDFILE_TOCTOU_REORACLE
PARENT_PRODUCT_TERMINAL = P8R-B
CURRENT_ADMITTED_REORACLE_TERMINAL = P8R-BR
OWNER_ADMISSION = ACCEPT_QUALIFIED
P8R_A_CLAIMED = NO
PRODUCT_MUTATION = NO
SOURCE_PROVENANCE_MODE = EXACT_BINARY_REPLAY
CLEAN_GIT_REPRODUCIBILITY = NO
WORKSPACE_GATES = NOT_RUN_INCOMPLETE_ATTRIBUTABLE_SOURCE
```

Additive dual-Linux **reoracle** of deferred Static runtime oracles under **MODE A exact binary replay**. Does **not** reopen `P8-STATIC-FIFO-001`, does **not** rewrite P8R-B / P8RDOC-A / P8RDOCINT-A history, and does **not** claim P8R-A.

Parent repair narrative: [v044-static-openat2-nonblock-special-file-repair.md](v044-static-openat2-nonblock-special-file-repair.md).
Evidence-accounting seal: [v044-static-p8rbr-evidence-accounting-documentation-integration-terminal-block.txt](v044-static-p8rbr-evidence-accounting-documentation-integration-terminal-block.txt).

## Authority runs

| Arch | RUN_ID | Host | Terminal |
| --- | --- | --- | --- |
| amd64 | `20260902T080000Z` | Netcup native | `TERMINAL_REORACLE_RUN=PASS` (`remote_run.log` ≡ `RESULTS.env`) |
| arm64 | `20260902T080000Z-arm64` | Oracle native | `TERMINAL_REORACLE_RUN=PASS` (`remote_run.log` ≡ `RESULTS.env`) |

```text
INVALID_RUN = 20260902T070000Z
INVALID_RUN_STATUS = INVALID
INVALID_RUN_REASON = remote_run.log_FAIL_then_offline_RESULTS_env_reparse
INVALID_RUN_MAY_SUPPORT_TERMINAL = NO
```

`remote_run.log` / `RESULTS.env` agreement is **same-harness internal consistency**, not independent corroboration.

## Provenance (MODE A)

```text
SOURCE_MANIFEST_SHA256 = 3556c3893b5f1cfcfde8fa338dda1207f3867b415efe34bc63f6022ad2451874
SOURCE_FILE_COUNT = 33
STATIC_SERVE_SOURCE_SHA256 = dc7965de88b18ba9981babb6cecb9d27e2ecd005e2b72a51cf83dd28f3b4db2e
AMD64_BINARY_SHA256 = eee3e4d93dd565988afc61be7cf731cc8a9c94f79c999eae2aec0f5a75fcaffe
ARM64_BINARY_SHA256 = 72652758ade208fb4d1685dd498077fd1c0d787370bfab9e6fb41bc019bfe918
HARNESS_SHA256 = ba4e79dbdcf7ad2796685af04c3326dbbbf15130eba4ee182cccc09fc9f60a9d
ATTRIBUTABLE_SOURCE_SNAPSHOT = PASS
CLEAN_GIT_REPRODUCIBILITY = NO
```

## Runtime oracles (authoritative artifacts)

| Claim | Arch | Raw artifact | Observed | Final | Qualification |
| --- | --- | --- | --- | --- | --- |
| Static small/binary/large hash | AMD64/ARM64 | `RESULTS.env` + body SHA files | PASS | PASS | per-run docroot |
| HEAD body bytes | both | `RESULTS.env` | PASS | PASS | |
| FIFO matrix (no-writer…races) | both | `RESULTS.env` / FIFO/* | PASS | PASS | causal path `FIFO_PATH_A` |
| FIFO alternate | both | `FIFO/alternate.txt` | 6/12 | REVIEW | not promoted to PASS |
| Real Linux sendfile | both | `STRACE/sendfile_calls.txt` | `sendfile(...)` ×2, 2MiB | PASS | syntax `sendfile` (not sendfile64) both arches |
| EAGAIN observed | both | `EAGAIN/strace_hits.txt` | 4× `-1 EAGAIN` | YES | full 2097152 + SHA match |
| Partial sendfile | both | `PARTIAL_SENDFILE/offsets.txt` | 6 positive / 10 syscalls; MONOTONIC | PASS | offsets bound to strace |
| TOCTOU 4000 | both | `TOCTOU/summary.txt` | OUTSIDE=0 | BOUNDED_RESIDUAL | not impossibility |
| Containment | both | `SECURITY/*` | outside/php/dot FAIL-closed | PASS | |
| Special-file bytes | both | `RESULTS.env` | 0 | 0 | |

### Sendfile / EAGAIN / partial (payload SHA)

| Arch | Sendfile calls | Length | Payload SHA256 |
| --- | --- | --- | --- |
| AMD64 | 2 | 2097152 | `4e2ae58a344bd6103798f604750486fecf1fe53857f59b2face919b7901cab73` |
| ARM64 | 2 | 2097152 | `c608832cb398dc7e4cc2028bbba7c8a4616a7e668788bba6ec794c9834393f91` |

Partial offset sequences (positive returns only; EAGAIN rows returned=-1 omitted from progress):

- AMD64 ends: `30 → 984960 → 1313280 → 1641600 → 1969920 → 2097152` (`MONOTONIC=True`)
- ARM64 ends: `36 → 986688 → 1315584 → 1644480 → 1973376 → 2097152` (`MONOTONIC=True`)

## Resource series reconciliation (critical)

Raw samples: `RESOURCES/series.txt` (both arches). Units:

- **FD** = `ls /proc/$DP_PID/fd | wc -l` (descriptor count)
- **RSS** = `/proc/$DP_PID/status` `VmRSS` (**KiB**)
- **THREADS** = `/proc/$DP_PID/status` `Threads`

| Series | AMD64 | ARM64 | Classification |
| --- | --- | --- | --- |
| FD | `12,12,12,12,12,12,12,12` | `12,12,12,12,12,12,12,12` | CAPTURED |
| RSS (KiB) | `3440,3540,3540,3540,3540,3540,3540,3540` | `2880,3124,3124,3124,3124,3124,3124,3124` | CAPTURED |
| THREADS | `8,8,8,8,8,8,8,8` | `8,8,8,8,8,8,8,8` | CAPTURED |
| CHILD | — | — | NOT_CAPTURED |
| SOCKET | — | — | NOT_CAPTURED |

```text
PRIOR_TERMINAL_ERROR = FD_SERIES_AMD64_WAS_MISLABELED_RSS_VALUES
CORRECTED_FD_SERIES_AMD64 = 12,12,12,12,12,12,12,12
FD_STAIRCASE = 0
FD_STAIRCASE_MEANING = max_minus_min_across_captured_tags_equals_0
RSS_TREND = STABLE_POST_WARMUP
RSS_TREND_CRITERION = after_PRE_to_baseline_step_remaining_tags_flat
THREAD_STAIRCASE = 0
THREAD_STAIRCASE_MEANING = Threads_constant_8_across_captured_tags
CHILD_STAIRCASE = NOT_PROVEN
SOCKET_STAIRCASE = NOT_PROVEN
RESOURCE_SERIES_ACCOUNTING = RECONCILED_WITH_RAW_EVIDENCE
UNSUPPORTED_ZERO_STAIRCASE_CLAIMS_REMOVED = YES
```

Prior detached terminal `573a89fb` incorrectly set `FD_SERIES_AMD64` equal to RSS and claimed `CHILD_STAIRCASE=0` / `SOCKET_STAIRCASE=0` without samples. Those unsupported zeros are **removed**.

Harness note: `RESULTS.env` hardcoded `THREAD_STAIRCASE=0` / `SOCKET_STAIRCASE=0` without deriving SOCKET from samples. THREAD constant-8 is corroborated by `series.txt`; SOCKET/CHILD remain **NOT_PROVEN**.

## Explicit non-closures

```text
FIFO_ALTERNATE = REVIEW_harness_oracle
SYMLINK_TOCTOU_STATUS = BOUNDED_RESIDUAL
TOCTOU_HASH_MISMATCHES = NOT_MEASURED
CONFIG_DISCLOSURE = NOT_MEASURED
ANTI_KOBAYASHI = NOT_MEASURED
PREEXISTING_FCGI_POOL_FLAKE = UNRESOLVED
FLAKE_ATTRIBUTION_EVIDENCE = NOT_EXECUTED
FMT/CHECK/CLIPPY/TESTS = NOT_RUN_INCOMPLETE_ATTRIBUTABLE_SOURCE
```

## Harness/evidence-path fixes (not product)

```text
SENDFILE_REGEX_FIX = sendfile64?_to_sendfile(64)?
TOCTOU_SENDFILE_ORDER_FIX = APPLIED
PROCESS_GROUP_FIX = setsid
SLOW_READER_TIMEOUT_SECONDS = 120
INVALID_OFFLINE_REPARSE_REJECTED = YES
```

## Evidence layout

```text
.exyonq-local/evidence/static-eagain-partial-sendfile-toctou-reoracle/
  20260902T080000Z/
  20260902T080000Z-arm64/
  p8rbrdoc-a-inventory/authoritative_artifact_inventory.csv
```

Historical detached draft commit (docs-only parent = canonical HEAD; resource claims superseded by this seal):

```text
SOURCE_DOC_COMMIT = 573a89fb45af7ec5e70e393ad23be64d0dc482c6
```
