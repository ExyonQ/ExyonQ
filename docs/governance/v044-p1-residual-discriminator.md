# V044 P1 residual causal discriminator

**WIP:** `V044_P1_RESIDUAL_CAUSAL_DISCRIMINATOR`  
**Product mutation:** NO  
**Evidence:** `.exyonq-local/benchmarks/v044-p1-residual-discriminator-20260820T182056Z/`  
**Product authority:** `427b1739` (+ `f2e90beb`); lab terminal `4b1a41f6` / tree `d7b5ba28`  
**Frozen priors:** SessionTable FIXED_CONFIRMED; static TX FIXED_NOT_PRIMARY_RPS_ROOT

## Outcome

```text
OUTCOME_A = HIGH_CONFIDENCE_SINGLE_ROOT
PRIMARY_RESIDUAL_ROOT = WAF_DEFAULT_MONITOR_ON_ABSENT_SECTION
PRIMARY_ROOT_CONFIDENCE = HIGH
RESIDUAL_MODEL = SINGLE_ROOT
RECOMMENDED_NEXT_PRODUCT_CHANGE = NONE
RECOMMENDED_NEXT_ACTION = HARNESS_OR_CONFIG_NORMALIZE_P1_RAW_EXPLICIT_WAF_OFF
```

**Successor (COMPLETE):** `docs/governance/v044-p1-raw-waf-normalization.md` — explicit RAW WAF-off + authoritative three-way → `P1_STATUS=CLOSED_CURRENT_COMPETITIVE_BASELINE` (CASE A).

## Phase 1 control (this session)

| Server | Median RPS | CV% |
| --- | --- | --- |
| ExyonQ | 138634.92 | 7.20 |
| NGINX | 143466.14 | 10.33 |
| OLS | 123086.28 | 9.35 |

```text
EXYONQ_VS_NGINX = -3.37%
EXYONQ_VS_OLS   = +12.63%
```

Prior post-SessionTable residual control (130421.60 / −20.57% vs NGINX) remains historical; this session’s CV is high. **WAF A/B** is the authoritative same-session causal test.

## Discriminator A — WAF RAW contract

### Config / code truth

| Field | Value |
| --- | --- |
| `P1_RAW_WAF_CONFIG` | `bench.toml` has **no** `[waf]` |
| `WAF_SECTION_PRESENT` | NO |
| `WAF_ENABLED_EFFECTIVE` | YES (`default_waf_enabled() = true`) |
| `WAF_MODE_EFFECTIVE` | MONITOR (`default_waf_mode() = Monitor`) |
| Live proof | continuous `EXY-HDR-1001` WARN under rewrk |
| Classification | `DEFAULT_MONITOR_SEMANTICS` + `ACTUAL_RULE_EVALUATION` |
| `RAW_BENCHMARK_CONTRACT_VIOLATION` | YES (if RAW claims WAF off) |
| Kind | harness assumption drift + expected product IR defaults |

### Causal A/B (workers=4, host cpuset 0–7, alternating)

| Arm | Config | Median RPS |
| --- | --- | --- |
| A | canonical (absent `[waf]` → Monitor) | **125494.16** |
| B | `[waf] enabled=false mode="disabled"` (diagnostic IR) | **164982.42** |

```text
WAF_RPS_DELTA_PCT = +31.47%
pair deltas = +44.15%, +27.64%, +20.49% (all B > A)
WAF_CAUSAL_SIGNAL = YES
```

Perf: A shows `lock_contended` → `Stdout::write_all` (~1.99%) consistent with Monitor WARN logging; B does not. This is **not** a product WAF “optimization” authorization — it is proof that **RAW ranking under default Monitor is not a WAF-off measurement**.

## Discriminator B — worker topology

Host CPUs fixed (`cpuset 0-7`). WAF-off diag to avoid WAF confound. `WORKER_TOPOLOGY_CHANGE_AUTHORIZED = NO`.

| Metric | workers=4 | workers=8 |
| --- | --- | --- |
| Alternating median RPS | 160161.26 | 143849.74 (−10.18%) |
| Busy tokio workers (sample) | 4 | 4 |
| Scale shape (single reps) | 4 → 129681 | 8 → 165815 |

```text
WORKER_TOPOLOGY_CAUSAL_SIGNAL = INCONCLUSIVE
RPS_BY_WORKERS = {1: 98420, 2: 120370, 4: 129681, 8: 165815}
```

Alternating A/B and sequential scale disagree; do not promote workers to HIGH.

## Discriminator C — openat2 / metadata

Under WAF-off workers=4:

```text
OPENAT2_PER_REQ ≈ 1.001
STATX_PER_REQ  ≈ 1.001
CLOSE_PER_REQ  ≈ 1.003
OPENAT2_RPS_IMPACT = NOT_PROVEN
FD_REUSE_REQUIRED  = NOT_PROVEN
```

Geometry confirmed; no product FD-cache A/B authorized → no HIGH RPS attribution.

## Syscall delta (WAF-off ExyonQ vs NGINX)

```text
EXYONQ ≈ 5.05 syscalls/req
NGINX  ≈ 4.09 syscalls/req
```

Extra / naming deltas: `openat2≈1`, `read≈1` (vs nginx `recvfrom`), `sendto≈1` (vs nginx `writev`). Count alone ≠ RPS proof.

## Secondary roots

- `WORKER_PARALLELISM_INCONCLUSIVE`
- `STATIC_METADATA_OPENAT2_GEOMETRY_CONFIRMED_RPS_NOT_PROVEN`

## Next step rule

Do **not** change product WAF defaults, worker defaults, openat2, or FD lifecycle without new owner authorization.

Authorized-class follow-up (config/harness only, still needs owner ACK to land in canonical P1 RAW):

```text
explicit [waf] enabled=false mode="disabled" in P1 RAW bench.toml
  OR equivalent documented RAW normalization
```

Then re-measure three-way control under true WAF-off before any residual product optimization or P2.

```text
P1_STATUS = OPEN_RESIDUAL_GAP_ANALYSIS
P2_STARTED = NO
PRODUCT_MUTATION = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
```
