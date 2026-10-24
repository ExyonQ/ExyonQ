# V044 P1 post-SessionTable residual reprofile

**WIP:** `V044_P1_POST_SESSIONTABLE_RESIDUAL_REPROFILE` (COMPLETE)  
**Successor:** `V044_P1_RESIDUAL_CAUSAL_DISCRIMINATOR` → `docs/governance/v044-p1-residual-discriminator.md`  
**Product mutation:** NO  
**Evidence:** `.exyonq-local/benchmarks/v044-p1-post-sessiontable-residual-20260820T132751Z/`  
**Product authority:** `f2e90beb` (absolute-form sendfile) + `427b1739` (thread-local SessionTable)  
**Lab terminal HEAD:** `4b1a41f6` / tree `d7b5ba28`

## Frozen prior roots (do not reopen without contradictory evidence)

| Item | Status |
|------|--------|
| `OLD_STATIC_TX_FUNCTIONAL_DEFECT` | `FIXED_AND_RUNTIME_PROVEN` |
| `OLD_STATIC_TX_RPS_ATTRIBUTION` | `FALSIFIED_OR_MATERIALLY_INCOMPLETE` |
| `OLD_SESSIONTABLE_MUTEX` | `FIXED_AND_CAUSALLY_CONFIRMED` |
| `OLD_SESSIONTABLE_RPS_EFFECT` | `CONFIRMED` (+103.4% same-session) |

Prior residual profile (pre-SessionTable): `docs/governance/v044-p1-residual-gap-reprofile.md`.  
SessionTable acceptance: `docs/governance/v044-p1-cap067-sessiontable-lock.md`.

## This-session control (canonical P1; supersedes prior post-fix medians for attribution)

| Server | Runs (RPS) | Median |
|--------|------------|--------|
| ExyonQ | 128892, 139909, 130422, 133763, 128349 | **130421.60** |
| NGINX | 164193, 167715, 163046, 148328, 169155 | **164192.77** |
| OLS | 141963, 135654, 142687, 156412, 137006 | **141962.77** |

```text
EXYONQ_VS_NGINX = -20.57%
EXYONQ_VS_OLS   = -8.13%
```

Historical post-fix control (143182 / 164066 / 140570, −12.7% / +1.9%) remains recorded but is **not** this session’s reference.

## Runtime verification

| Check | Result |
|-------|--------|
| Binary SHA-256 | `5a31ccf3decabed3071e587ad1c68fa5fb673f2b130ef8f63e70f7a5c0a13c40` |
| Rust / Cargo | 1.97.1 |
| Cargo.lock SHA-256 | `6086f47d…bef6` unchanged |
| Global SessionTable mutex | **NO** (`thread_local!` present) |
| SessionTable hot sample | **0** |
| Sendfile under absolute-form | **YES** (≈1.00/req) |
| openat2/req | ≈1.00 |
| Benchmark-only product branch | **NO** |

## Syscall geometry (retry with worker PIDs)

| Server | syscalls/req | Dominant |
|--------|--------------|----------|
| ExyonQ | **7.847** | futex≈1.70, read≈1.05, close≈1.01, write+sendto+sendfile+openat2 ≈1 each |
| NGINX | **4.092** | close+recvfrom+writev+sendfile ≈1 each; openat2=0; futex≈0 |
| OLS | INSUFFICIENT | attach did not capture hot workers |

Questions answered:

1. After SessionTable removal, ExyonQ still runs ~**1.9×** NGINX syscalls/req (was ~2.8× pre-fix at 11.5).  
2. openat2+close remain **material** (≈2/req on ExyonQ; NGINX has close≈1 but **no** openat2).  
3. Residual futex ≈1.70/req — **not** SessionTable; shares include non-SessionTable `lock_contended` (~1.5%), WAF regex pool (~3.2%), kernel spinlocks (mis-tagged “tokio” by thread name).  
4. Syscall delta alone is **plausible as a component** of a ~20% gap, not proven as the sole root.

## Workers / CPU

| Measure | Value |
|---------|-------|
| Configured / active / idle | 4 / 4 / ~4–5 idle tokio threads |
| RPS @ 1/2/4/8 host CPUs | 40435 / 78998 / 140792 / 135274 |
| Scaling 4→8 host CPUs | **−3.92%** (flat) |
| Diagnostic WORKER4 → WORKER8 | 135665 → **145709** (**+7.40%**) |
| `WORKER_TOPOLOGY_CAUSAL_SIGNAL` | YES |
| `WORKER_TOPOLOGY_CHANGE_AUTHORIZED` | **NO** |

## WAF / tracing

| Item | Result |
|------|--------|
| WAF keys in bench.toml | none |
| WAF required in P1 RAW | **NO** |
| regex_automata / exyonq_waf samples | **~3.2%** (accidental path under RAW-off — contract question, not authorization to optimize) |
| Access log / OTEL | disabled |
| Tracing stdout share | ~0.91% → `TRACING_OPTIMIZATION_REQUIRED=NO` |

## Measurement caveats

- Cross-server `perf stat` absolute cycles/instructions/req: **WEAK_OR_INVALID** (ExyonQ showed ~33 CPUs utilized with 4 active workers).  
- OLS syscall attach: **INSUFFICIENT**.  
- Network softirq/retransmit: **INSUFFICIENT** (no invented host-global attribution).

## New residual causal ranking

| Rank | Cause | Confidence | Product optimizable |
|------|-------|------------|---------------------|
| 1 | Syscall geometry (openat2 + futex + MSG_MORE write/sendto) | MEDIUM | PARTIAL |
| 2 | Worker parallelism (worker=8 +7.4% diagnostic) | MEDIUM / PARTIAL | YES but unauthorized |
| 3 | Static metadata path (~8% samples) | MEDIUM; FD reuse **NOT_PROVEN** | PARTIAL |
| 4 | WAF regex pool despite RAW-off (~3.2%) | PARTIAL | contract first |
| 5 | Tracing stdout | FALSIFIED as root | NO |

```text
PRIMARY_RESIDUAL_ROOT_IDENTIFIED = NO
RECOMMENDED_NEXT_PRODUCT_CHANGE = NONE
P1_STATUS = OPEN_RESIDUAL_GAP_ANALYSIS
P2_STARTED = NO
OWNER_AUTHORIZATION_REQUIRED_BEFORE_NEXT_OPTIMIZATION_OR_P2 = YES
```

## Falsification accounting

```text
STATIC_TX_CURRENT_STATUS = FIXED_NOT_PRIMARY_RPS_ROOT
SESSIONTABLE_CURRENT_STATUS = FIXED_CONFIRMED_CAUSAL_ROOT
FD_REUSE_HYPOTHESIS = NOT_PROVEN
WORKER_TOPOLOGY_HYPOTHESIS = PARTIAL
WAF_REGEX_HYPOTHESIS = PARTIAL
TRACING_HYPOTHESIS = FALSIFIED
```

## Successor discriminator (COMPLETE)

See `docs/governance/v044-p1-residual-discriminator.md`.

```text
OUTCOME_A = HIGH_CONFIDENCE_SINGLE_ROOT
PRIMARY_RESIDUAL_ROOT = WAF_DEFAULT_MONITOR_ON_ABSENT_SECTION
PRIMARY_ROOT_CONFIDENCE = HIGH
WAF_CAUSAL_SIGNAL = YES (+31.47% same-session A/B)
WORKER_TOPOLOGY_CAUSAL_SIGNAL = INCONCLUSIVE
OPENAT2_RPS_IMPACT = NOT_PROVEN
RECOMMENDED_NEXT_PRODUCT_CHANGE = NONE
RECOMMENDED_NEXT_ACTION = HARNESS_OR_CONFIG_NORMALIZE_P1_RAW_EXPLICIT_WAF_OFF
```
