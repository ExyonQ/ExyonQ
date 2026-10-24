# V044 P1 — Final post-repair authoritative rebaseline & seal

```text
STATUS = COMPLETE
CASE = S1
WIP = V044_P1_FINAL_POST_REPAIR_REBASELINE_AND_SEAL
MODE = READ_ONLY_AUTHORITATIVE_REBASELINE
PRODUCT_MUTATION = NO
PRODUCT_CODE_CHANGED = NO
P2_STARTED = NO
USERSPACE_RESIDUAL_ATTRIBUTION_STARTED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

PRODUCT_AUTHORITY_HEAD = 06742e1bdc67b5ed86ef95750399e9edad16d646
PRODUCT_AUTHORITY_TREE = 5818095793373bd322ac13a2b3ebcf32661941e1
DOCKER_EXYONQ_BINARY_SHA256 = 09301dc867a4161ebf1da24635f97df14acffcc25f8211b1e475b97b6defe36f
HOST_SANITY_BINARY_SHA256 = 4614d5448ef69c9697c982b482f155cc6cf0abe143675dab9ef57ade5a927da0
  # same product commit; arch/path distinct hashes expected (Docker vs host target)

EVIDENCE_NETCUP =
  /root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p1-cpu-live-3way-20260821T191819Z/
LAB_COPY =
  .exyonq-local/evidence/v044-p1-final-post-repair-rebaseline-20260821T191819Z/
```

## Authority note

Lab `HEAD` may advance for unrelated governance (e.g. claim/evidence guard). **Product
measurement authority for this seal is git-archive of `06742e1b`** (tree
`58180957…`). Product paths are identical between `06742e1b` and later integrity-only
commits at the time of measurement.

Ambient dirty worktree was **not** used to build the P1 image.

## Historical baselines (preserved — not current truth)

| Epoch | ExyonQ RPS | NGINX RPS | OLS RPS | CPU cost vs NGINX | CPU cost vs OLS |
|-------|------------|-----------|---------|-------------------|-----------------|
| Live-3way Root1+Root3 (pre-FD) | ≈150318 | ≈158754 | ≈132839 | **−2.94%** | **+28.56%** |
| Post-FD-reuse (pre-H1) | ≈180108 | ≈157198 | ≈131326 | **−22.32%** | **+4.02%** |

```text
DO_NOT_CARRY_FORWARD_AS_CURRENT =
  EXYONQ_CPU_COST_VS_OLS_PERCENT_AFTER = +4.02%
  USERSPACE_GAP = 4.35 us/request   # pre-FD attribution meter; not post-repair residual
```

## Contract (unchanged)

Static 1 KiB · HTTP/1.1 keepalive · 100 connections · 2 rewrk threads · 20s warmup ·
30s measure · 5 reps · cpuset 0–7 · CpuQuota=0 · Docker internal · RAW WAF explicit OFF
(`p1-raw-waf-off-gate.sh` PASS).

Harness: `scripts/remote/v044-p1-cpu-live-3way.sh`

## Comparability

```text
P1_COMPARABILITY = PASS
EXYONQ_CPUSET = NGINX_CPUSET = OLS_CPUSET = 0-7
EXYONQ_CPU_QUOTA = NGINX_CPU_QUOTA = OLS_CPU_QUOTA = 0
EXYONQ_EXECUTION_MODEL = NGINX_EXECUTION_MODEL = OLS_EXECUTION_MODEL = Docker compose v044p1auth-clean
LOAD_GENERATOR = rewrk (bench-runner)
RIVAL_CONFIGS_CANONICAL = YES (established P1 authoritative overlay)
HOST_INTERFERENCE = OBSERVED
  # kud2-nginx / kud3-nginx / extra bench-runner present; disk ~95%
  # P1 trio remained symmetric; probes 200/1024 for all three
```

## Current docker-stats three-way (authoritative live meter)

| | ExyonQ | NGINX | OLS |
|--|--|--|--|
| RPS median | **141937.78** | **162268.07** | **138943.97** |
| RPS CV | 0.133 | 0.195 | 0.123 |
| CPU avg % | 163.50 | 222.22 | 146.06 |
| core-ms/req | **0.011519** | **0.013694** | **0.010512** |
| P50 / P99 (ms) | 0.86 / **1.81** | 0.70 / **1.43** | 0.96 / **1.70** |

```text
EXYONQ_CPU_COST_VS_NGINX_PERCENT = -15.89%
EXYONQ_CPU_COST_VS_OLS_PERCENT = +9.58%
EXYONQ_RPS_VS_NGINX_PERCENT = -12.53%
EXYONQ_RPS_VS_OLS_PERCENT = +2.15%
```

RPS repetitions:

```text
EXYONQ = [117138.22, 165776.87, 161691.98, 141388.60, 141937.78]
NGINX  = [102834.90, 174341.72, 142990.14, 170962.55, 162268.07]
OLS    = [159043.40, 117083.25, 129625.01, 138943.97, 153578.93]
```

## Cgroup cpu.stat companion (separate meter — not forced equal to docker-stats)

Single 20s warmup + 30s measure per server after the series:

| | total µs/req | user µs/req | system µs/req |
|--|--|--|--|
| ExyonQ | 32.619 | 6.661 | 25.959 |
| NGINX | 39.275 | 6.998 | 32.276 |
| OLS | 30.996 | 9.339 | 21.657 |

Direction agrees with docker-stats: ExyonQ cheaper than NGINX; slightly above OLS on total µs/req (~+5.2% this window).

```text
EXYONQ_INSTRUCTIONS_PER_REQ = NOT_MEASURED  # live-3way harness has no perf counters
EXYONQ_BRANCHES_PER_REQ = NOT_MEASURED
```

## Cap067 non-regression (host binary from same product commit)

```text
FD_REUSE_HIT_PATH = PASS
OPENAT2_CALLS_PER_REQ ≈ 0
NEWFSTATAT_CALLS_PER_REQ ≈ 1
CLOSE_CALLS_PER_REQ ≈ 0.1
SENDFILE_CALLS_PER_REQ ≈ 1  # sendfile_total=20 / n=20
CAP067_MISS_SANITY = PASS  # never_existed / delete → 404
CAP067_TRAVERSAL_SANITY = PASS  # 403
CAP067_SYMLINK_SANITY = PASS  # 403
CAP067_MISS_NEEDHYPER_HANG_STATUS = CLOSED_FIXED_AND_VERIFIED_REAL_PRODUCTION
FD_GEOMETRY_ROOT = CLOSED
FD_REUSE_REPAIR_RESULT = PROVEN_MATERIAL_SUCCESS
```

## Classification

```text
P1_EXYONQ_VS_NGINX = AHEAD
  # core-ms/req −15.89%; cgroup total µs also below NGINX
P1_EXYONQ_VS_OLS = COMPETITIVE_PARITY
  # docker-stats +9.58% / cgroup ~+5%; single-digit soft band; RPS CV high;
  # not treated as a new material causal defect requiring product WIP
P1_OVERALL_COMPETITIVE_CLASSIFICATION = CLOSED_CURRENT_COMPETITIVE_BASELINE
MATERIAL_REPEATABLE_GAP = NO
P1_FINAL_STATUS = CLOSED_CURRENT_COMPETITIVE_BASELINE
P1_CPU_EFFICIENCY_STATUS = CLOSED_COMPETITIVE
P1_OPEN_CAUSAL_ROOTS = 0
NEXT_CAUSAL_WIP_PROPOSAL = NONE
```

### Commercial / migration axes (independent — do not collapse)

Per `docs/governance/performance-competitive-admission.md` and scoreboard
`docs/benchmarks/v044-p1-p14-competitive-dashboard.json`:

```text
P1_ENGINEERING_BASELINE = CLOSED
P1_PERFORMANCE_COMPETITIVENESS = MIXED
P1_COMPETITIVE_WIN = NOT_PROVEN
P1_MIGRATION_VALUE_VS_NGINX = NOT_PROVEN
P1_MIGRATION_VALUE_VS_OLS = NOT_PROVEN
P1_MARKET_READINESS = NOT_READY
```

Winning CPU/request vs NGINX alone does **not** prove competitive win or migration value
(ExyonQ loses RPS, P50/P95/P99, and uses ~2× RSS vs NGINX on this seal).

Do **not** reopen FD geometry / FD-reuse / H1 miss hang on this evidence.
Do **not** start P2 or userspace residual attribution automatically.

## Integrity

```text
ZERO_FAKE = YES
USES_REAL_DATA = YES
CHANGES_PRODUCT_SEMANTICS = NO
ADDS_BENCHMARK_SPECIAL_CASE = NO
GLOBAL_CLIPPY_D_WARNINGS = NOT_PROVEN_DUE_TO_PREEXISTING_UNRELATED_STATE
CAP067_MISS_REPAIR_CLIPPY_D_WARNINGS = PASS  # prior WIP; not reopened
```

```text
STOP FOR OWNER REVIEW
PUSH = NO
PUBLICATION = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
```
