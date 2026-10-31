# V044 P3 Cap067 — large-body residual causal profile (post cork + 128 KiB KEEP)

```text
STATUS = COMPLETE
WIP = V044_P3_CAP067_LARGE_BODY_CAUSAL_PROFILE
MODE = READ_ONLY_CAUSAL_MEASUREMENT
PRODUCT_MUTATION = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

HOST = netcup-bench (amd64)
SUITE = v044-p3-cap067-large-body-causal-20260917T143741Z
EVIDENCE_NETCUP =
  /root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p3-cap067-large-body-causal-20260917T143741Z/
LAB_COPY =
  .exyonq-local/evidence/v044-p3-cap067-large-body-causal-20260917T143741Z/
BINARY_SHA256 = fefe547d13a056de36fc43a5fab1feac8bc7ab957e3a65db318689806aa49d7c
KEEP_STATE = Cap067 cork + EXYONQ_SENDFILE_CHUNK=131072 + geom 8/8/8
HARNESS = scripts/remote/v044-p3-cap067-large-body-causal-netcup.sh
```

## Phase A — cgroup µs/req (authoritative gap)

Clean window (no bpf/perf observers), P3 `/site/1m.bin`, c=100 t=2, warm 20s / measure 30s.

| | ExyonQ | NGINX |
| --- | --- | --- |
| RPS (window) | 5603 | 7104 |
| user µs/req | **10.6** | 16.4 |
| system µs/req | **617.3** | 147.1 |
| total µs/req | **627.9** | 163.4 |

```text
GAP_TOTAL_US_PER_REQ   = 464.5
GAP_SYSTEM_US_PER_REQ  = 470.2
GAP_USER_US_PER_REQ    = -5.8   # ExyonQ cheaper in userspace
SYSTEM_SHARE_OF_GAP    = ~100%
GAP_CLASS              = SYSTEM_DOMINATED
```

**Interpretation:** Cap067 userspace FSM is **not** the residual. The entire competitive deficit on this meter is **kernel/system time** (~4.2× NGINX system µs/req).

## Phase B — bpftrace

```text
BPF_STATUS = FAILED_SETUP
REASON = tracepoint syscalls:sys_enter_sendfile absent on this kernel
         (script also attached sendfile64; load aborted on first missing TP)
SYSCALL_GEOMETRY = NOT_MEASURED_THIS_RUN
```

Do not treat empty bpf maps as “zero sendfile”. Re-run with sendfile-only TPs if count geometry is needed.

## Phase C — perf (symbol categories, self%)

Captured top-symbol mass (parser of `--percent-limit 0.4` report):

| Category | ExyonQ | NGINX |
| --- | --- | --- |
| SPINLOCK | **13.9%** | 13.7% |
| MEMCG (`page_counter*`) | **4.5%** | 3.0% |
| TCP_SKB | 2.3% | **11.7%** |
| SENDFILE_IO (`filemap`/`splice`/`sendfile`) | 1.9% | **10.5%** |

ExyonQ hot samples skew to **spinlock + memcg charge** with little productive sendfile/TCP work in the top list. NGINX shows the expected zero-copy path (`filemap_get_read_batch`, `skb_append_pagefrags`, `splice_folio_into_pipe`).

```text
PERF_HINT_PRIMARY = LOCK_AND_MEMCG_TAX_UNDER_PARALLEL_LARGE_SEND
PERF_HINT_SECONDARY = LESS_PRODUCTIVE_SENDFILE_SHARE_THAN_NGINX
```

## Causal ranking

1. **Kernel/system cost** (cgroup) — primary, proven  
2. **Spinlock / memcg charge pressure under parallel large-body send** — supported by perf  
3. **skb_split / page path** — present on ExyonQ (`skb_split` in top), consistent with prior Cap067 tax narrative  
4. **Userspace Cap067 / header / WAF** — falsified as residual (user µs better than NGINX)

## Forbidden next moves

- Reintroduce `TCP_NOTSENT_LOWAT`, `SO_SNDBUF=2MiB`, uncork-after-header  
- Raise `SENDFILE_MIN`  
- More userspace FSM micro-opts as primary P3 play  
- Claiming competitive win / publishing HTML from this WIP  

## Next product hypothesis (single)

```text
NEXT_PRODUCT_HYPOTHESIS =
  GEOMETRY_CONTENTION_AB_ON_P3
  # Same KEEP binary; reduce Cap067 parallel senders (e.g. EPOLL_POOL 8→4
  # and/or ACCEPT_WORKERS 8→4) under P3 load; measure system µs/req + RPS.
  # Protectors: P1/P2 RPS drop ≤3%, p99 worsen ≤10%.
  # KEEP only if P3 system µs/req drops materially and protectors PASS.

STATUS_FOLLOWUP = FALSIFIED
  # Suite v044-p1p3-geometry-contention-ab-20260917T144507Z
  # See docs/governance/v044-p3-cap067-geometry-contention-ab.md
  # P3 RPS −1.2%, core-ms +4.4%; P2 protector FAIL (−8.6% RPS). KEEP geom8.

ALTERNATE_IF_FALSIFIED =
  MEMCG_PACKAGING_RESEARCH = DEFERRED_INFEASIBLE (root memory controller on Netcup)
  SENDFILE_PRODUCTIVITY_DIAG = COMPLETE (suite 20260917T191201Z)
    # ExyonQ 8×128KiB sendfile64/req vs NGINX 1×1MiB; SENDFILE_IO% 0.58 vs 12.3
  NEXT_PRODUCT_HYPOTHESIS =
    CAP067_LARGE_BODY_CORK_OFF_AB (env EXYONQ_LARGE_BODY_TCP_CORK=0)
  deeper sendfile vs nginx splice semantics (may need ADR; splice still MVP-out)
```

## Complexity

```text
Complexity Added:   harness + this note
Complexity Removed: 0
Net Complexity:     +small (diagnostic only)
```
