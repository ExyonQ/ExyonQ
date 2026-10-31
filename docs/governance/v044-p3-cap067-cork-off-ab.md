# V044 P3 Cap067 — large-body cork-off A/B

```text
STATUS = COMPLETE
WIP = V044_P3_CAP067_LARGE_BODY_CORK_OFF_AB
PRODUCT_MUTATION = YES (env kill-switch EXYONQ_LARGE_BODY_TCP_CORK; default remains ON)
VERDICT_B = FALSIFIED_FOR_P3_RPS
VERDICT_C = LIFT_VS_A_BUT_COMPETITIVE_DOD_UNMET
DEFAULT_KEEP = cork ON + EXYONQ_SENDFILE_CHUNK=131072 + geom 8/8/8
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

SUITE = v044-p1p3-cork-off-ab-20260917T192047Z
HOST = netcup-bench
IMAGE = v044p1auth-exyonq-corkab
BINARY_SHA256 = e51bb1293cad97d3bed2370695193ab2499f38f19327a55d05d0d6b4e5e6faa5
EVIDENCE_NETCUP =
  /root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p1p3-cork-off-ab-20260917T192047Z/
LAB_COPY =
  .exyonq-local/evidence/v044-p3-cap067-cork-off-ab-20260917T192047Z/
PRIOR = docs/governance/v044-p3-cap067-sendfile-productivity-diag.md
```

## Hypothesis

Productivity diag: ExyonQ ~8×128 KiB `sendfile64` under TCP_CORK vs NGINX ~1×1 MiB
without `tcp_nopush`. Uncapped **with cork** already lost to 128 KiB+cork. Test
large-body cork OFF (MSG_MORE per step).

## Arms (same image)

| Arm | Cork (large body) | Chunk |
| --- | --- | --- |
| A KEEP | ON | 128 KiB |
| B | OFF (`EXYONQ_LARGE_BODY_TCP_CORK=0`) | 128 KiB |
| C diagnostic | OFF | uncapped (1 MiB) |

## ExyonQ medians

| Arm | P1 RPS | P1 p99 | P2 RPS | P2 p99 | P3 RPS | P3 p99 | P3 core-ms |
| --- | --- | --- | --- | --- | --- | --- | --- |
| A | 148243 | 1.56 | 85913 | 4.91 | **5996** | 32.04 | 0.424 |
| B | 149134 | 1.61 | 93170 | 2.60 | 5996 | 33.32 | 0.395 |
| C | 146958 | 1.53 | 92010 | 3.02 | **6300** | 31.49 | 0.387 |

### Δ vs A

| Arm | P1 dRPS/dP99 | P2 dRPS/dP99 | P3 dRPS/dP99/dCMS |
| --- | --- | --- | --- |
| B | +0.6% / +3.2% | +8.5% / −47% | **−0.0%** / +4.0% / −6.8% |
| C | −0.9% / −1.9% | +7.1% / −38% | **+5.1%** / −1.7% / −8.7% |

### Protectors vs A

```text
B_P1 = PASS
B_P2 = PASS
C_P1 = PASS
C_P2 = PASS
```

### Competitive residual (P3 vs NGINX, same suite)

| Arm | vs NGINX |
| --- | --- |
| A | −22.2% |
| B | −24.7% |
| C | −24.3% |

```text
COMPETITIVE_DOD_MET = NO
```

## Verdict

```text
ARM_B_CORK_OFF_128K =
  FALSIFIED as P3 RPS attack (flat vs A).
  Incidental P2 lift; not sufficient to change Cap067 large-body cork default.

ARM_C_CORK_OFF_UNCAPPED =
  Improves ExyonQ P3 +5.1% vs A with protectors PASS and better core-ms.
  Does NOT close NGINX/OLS gap (still ~−24%).
  Diagnostic-not-default per admission — NOT promoted to product KEEP this WIP.
  Re-admit required before shipping cork-off+uncapped as default
  (contradicts prior uncapped+cork KEEP rejection; cork-off is the material change).

DEFAULT_KEEP_UNCHANGED =
  cork ON + 128 KiB clamp + geom 8/8/8
```

## Next

1. **Do not** claim P1–P3 competitive win from this suite.  
2. Cheapest remaining product path after cork geometry: **L3 splice / sendfile-path ADR** (NGINX `splice_direct` productivity) — only after explicit admission.  
3. Optional: re-admit Arm C as development KEEP if absolute ExyonQ P3 lift is the goal (still loses to NGINX).

## Complexity

```text
Complexity Added:   EXYONQ_LARGE_BODY_TCP_CORK env + harness + this note
Complexity Removed: 0
Net Complexity:     +small (ops kill-switch; default unchanged)
```
