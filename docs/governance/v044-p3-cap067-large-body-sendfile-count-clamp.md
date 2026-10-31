# V044 P3 Cap067 — large-body sendfile count clamp (skb_split semantic)

```text
WIP = V044_P3_CAP067_LARGE_BODY_SENDFILE_COUNT_CLAMP
STATUS = KEEP_128KIB
PRODUCT_MUTATION = YES
MEASUREMENT_CLASS = DEVELOPMENT
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
SUITE = v044-p1p3-sendfile-chunk-ab-20260916T200038Z
HOST = netcup-bench (amd64)
IMAGE = v044p1auth-exyonq-chunkab
BINARY_SHA256 = fefe547d13a056de36fc43a5fab1feac8bc7ab957e3a65db318689806aa49d7c
GEOMETRY = accept/worker/epoll = 8/8/8
EVIDENCE_NETCUP =
  /root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p1p3-sendfile-chunk-ab-20260916T200038Z/
LAB_COPY =
  .exyonq-local/evidence/v044-p3-cap067-sendfile-chunk-ab-20260916T200038Z/
```

## Hypothesis

After TCP_CORK KEEP, unbounded `sendfile64(count ≈ remaining)` for 1 MiB bodies may
inflate skb_split / page-batch work. Cap each large-body step count.

## Mutation (product KEEP)

| Item | Value |
| --- | --- |
| Gate | `remaining >= LARGE_BODY_MIN_BYTES` (= `TCP_CORK_MIN_BODY` = 65537) |
| Default clamp | `LARGE_BODY_SENDFILE_COUNT_MAX` = **128 KiB** (KEEP arm C) |
| P1 / P2 | Uncapped (MSG_MORE unchanged) |
| TCP_CORK | Unchanged (full-body cork until Complete) |
| Ops override | `EXYONQ_SENDFILE_CHUNK` (≥ 64 KiB) |
| `SENDFILE_MIN_BYTES` | **0** |

## Netcup live-3way A/B (same binary)

Arms:

| Arm | Policy |
| --- | --- |
| A | `EXYONQ_SENDFILE_CHUNK=1048576` (uncapped for P3) |
| B | unset → product **256 KiB** candidate |
| C | `EXYONQ_SENDFILE_CHUNK=131072` (**128 KiB**) |

### ExyonQ medians

| Arm | P1 RPS | P1 p99 | P2 RPS | P2 p99 | P3 RPS | P3 p99 | P3 core-ms |
| --- | --- | --- | --- | --- | --- | --- | --- |
| A uncapped | 141349 | 1.35 | 91987 | 2.69 | 6038 | 33.43 | 0.4087 |
| B 256 KiB | 145271 | 1.42 | 94903 | 2.70 | 6189 | 31.85 | 0.4062 |
| C 128 KiB | 142944 | 1.47 | 91971 | 2.74 | **6400** | 34.93 | **0.3963** |

### Δ vs A (ExyonQ)

| Arm | P1 dRPS / dP99 | P2 dRPS / dP99 | P3 dRPS / dP99 / dCMS |
| --- | --- | --- | --- |
| B | +2.8% / +5.2% | +3.2% / +0.4% | +2.5% / −4.7% / −0.6% |
| C | +1.1% / +8.9% | −0.0% / +1.9% | **+6.0%** / +4.5% / **−3.0%** |

### Protectors (vs A)

Gate: P1/P2 RPS drop ≤ 3%; p99 worsen ≤ 10%.

```text
B_PROTECTORS = PASS
C_PROTECTORS = PASS   # P1 p99 +8.9% within 10%
```

### Competitive residual (not closed)

| Arm | P3 vs NGINX | P3 vs OLS |
| --- | --- | --- |
| A | −22.9% | −27.7% |
| B | −22.8% | −24.4% |
| C | −21.0% | −24.5% |

```text
COMPETITIVE_DOD_MET = NO
NEXT = further large-body path work (not reintroduce invalidated socket knobs)
```

## KEEP decision

```text
KEEP = YES
KEEP_CLAMP = 128 KiB
REJECTED_CANDIDATE = 256 KiB (weaker P3 lift)
REVERT = NO
RATIONALE =
  Arm C best P3 RPS (+6% vs uncapped) with protectors PASS and slight core-ms win.
  Does not close NGINX/OLS gap; development KEEP only.
```

## Invalidated (do not reintroduce)

- `TCP_NOTSENT_LOWAT=256KiB`
- `SO_SNDBUF=2MiB`
- uncork-after-header
- harness-only memcg escape as product KEEP
- host-glibc `docker cp` of Netcup-host-built binary into bookworm image (use Docker build)

## Complexity

```text
Complexity Added:   size-gated default in sendfile_request_count (+ tests + this note)
Complexity Removed: 0
Net Complexity:     +small (justified by measured P3 lift without protector fail)
```
