# V044 P3 Cap067 — sendfile path productivity diagnostic

```text
STATUS = COMPLETE
WIP = V044_P3_CAP067_SENDFILE_PATH_PRODUCTIVITY_DIAG
PRODUCT_MUTATION = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

SUITE = v044-p3-cap067-sendfile-productivity-20260917T191201Z
HOST = netcup-bench
KEEP = cork + EXYONQ_SENDFILE_CHUNK=131072 + geom 8/8/8
BINARY_SHA256 = fefe547d13a056de36fc43a5fab1feac8bc7ab957e3a65db318689806aa49d7c
EVIDENCE_NETCUP =
  /root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p3-cap067-sendfile-productivity-20260917T191201Z/
LAB_COPY =
  .exyonq-local/evidence/v044-p3-cap067-sendfile-productivity-20260917T191201Z/
HARNESS = scripts/remote/v044-p3-cap067-sendfile-productivity-diag-remote.sh
```

## Memcg packaging (prior alternate)

```text
MEMCG_PACKAGING = DEFERRED_INFEASIBLE_ROOT_MEMORY_CONTROLLER
```

Root `cgroup.subtree_control` includes `memory` on Netcup. Every child cgroup
(including `systemd-run MemoryAccounting=no`) still exposes `memory.max`. True
“outside memory controller” A/B would require host-wide `subtree_control` surgery —
not admitted.

## Measured geometry (sendfile64-only bpf)

| | ExyonQ | NGINX |
|--|--------|-------|
| sendfile64 / req | **8.005** | **1.009** |
| avg `count` arg | **131 067** (~128 KiB) | **1 042 774** (~1 MiB) |
| SENDFILE_IO self% | **0.58** | **12.29** |
| RPS (cgroup window) | 6103 | 7681 (−20.5%) |
| system µs/req | **601** | **151** |

```text
GAP_CLASS = SYSTEM_DOMINATED
ESCALATION_HINT = EXYONQ_MORE_SENDFILE_STEPS_THAN_NGINX
```

NGINX uses one full-body `sendfile64` (kernel path shows `splice_direct_to_actor`).
ExyonQ KEEP clamp yields eight 128 KiB steps. Prior chunk A/B already showed
**uncapped+cork worse than 128 KiB+cork** for ExyonQ — do not “just uncap”.

nginx.conf: `sendfile on;` — **no** `tcp_nopush`.

## Next product hypothesis (admitted)

```text
NEXT = CAP067_LARGE_BODY_CORK_OFF_AB
  Arm A: KEEP cork ON + 128 KiB + geom8
  Arm B: EXYONQ_LARGE_BODY_TCP_CORK=0 + 128 KiB + geom8
  Arm C (diagnostic): cork OFF + uncapped (not default KEEP candidate)
```

Splice ADR remains deferred until cork-off A/B completes.

## Forbidden (reaffirmed)

- `TCP_NOTSENT_LOWAT`, `SO_SNDBUF=2MiB`, uncork-after-header-only
- Raise `SENDFILE_MIN`, geom4 default, public HTML claims
- Host memcg strip as product KEEP

## Complexity

```text
Complexity Added:   harness + this note
Complexity Removed: 0
Net Complexity:     +small (diagnostic)
```
