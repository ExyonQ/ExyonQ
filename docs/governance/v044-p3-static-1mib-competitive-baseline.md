# V044 P3 — Static 1 MiB competitive admission baseline

```text
STATUS = COMPLETE
CASE = P3-C1
WIP = V044_P3_STATIC_1MIB_COMPETITIVE_ADMISSION_BASELINE
MODE = COMPETITIVE_MEASUREMENT_ONLY
PRODUCT_MUTATION = NO
PRODUCT_CODE_CHANGED = NO
PERFORMANCE_REPAIR = NOT_AUTHORIZED
P4_STARTED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

PRODUCT_AUTHORITY_HEAD = 06742e1bdc67b5ed86ef95750399e9edad16d646
PRODUCT_AUTHORITY_TREE = 5818095793373bd322ac13a2b3ebcf32661941e1
PRODUCT_BINARY_SHA256 = 09301dc867a4161ebf1da24635f97df14acffcc25f8211b1e475b97b6defe36f

EVIDENCE_NETCUP =
  /root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p3-static-1mib-3way-20260821T224438Z/
LAB_COPY =
  .exyonq-local/evidence/v044-p3-static-1mib-3way-20260821T224438Z/
HARNESS = scripts/remote/v044-p3-static-1mib-live-3way.sh
```

## Contract reconciliation

```text
P3_CONTRACT_RECONCILIATION_REQUIRED = NO
P3_CONTRACT_SOURCE =
  benchmarks/scenarios/perf/scenarios.toml [p3]
  + P1/P2 live-3way competitive meter
P3_PAYLOAD_BYTES = 1048576
P3_PROTOCOL = HTTP/1.1
P3_KEEPALIVE = rewrk connection reuse
P3_CONNECTIONS = 100
P3_LOAD_THREADS = 2
P3_WARMUP_SECONDS = 20
P3_MEASURE_SECONDS = 30
P3_REPETITIONS = 5
P3_CPUSET = 0-7
P3_CPU_QUOTA = 0
P3_WAF_MODE = explicit enabled=false mode=disabled (RAW)
P3_EXECUTION_MODEL = Docker compose v044p1auth-clean + p1-authoritative overlay
P3_PATH = /site/1m.bin
P3_COMPARABILITY = PASS
```

## Correctness sanity

```text
GET_1MIB_STATUS = 200
GET_1MIB_BYTES = 1048576
GET_1MIB_HASH_MATCH = YES
  hash = 8f990ba0b577b51cf009ea049368c16bbda1b21e1b93be07a824758bb253c39b
KEEPALIVE_1MIB = PASS
HEAD_1MIB = PASS
TRUNCATED_RESPONSES = 0
HASH_MISMATCHES = 0
```

## CAUSE3C / geometry observation

```text
UID_ACCOUNTING_ASYMMETRY = YES   # ExyonQ uid=10001; rivals nginx/nobody workers
CAUSE3C_ASYMMETRY_PRESENT = YES  # UID present; not proven causal this run
PIPE_SOFT_BUDGET_STATE = pipe-max-size=1048576 (exyonq ns)
SPLICE_PIPE_GEOMETRY_ANOMALY = NO
SPLICE_ITERATION_BYTES = NOT_MEASURED
```

## Authoritative three-way (docker-stats meter)

| | ExyonQ | NGINX | OLS |
|--|--|--|--|
| RPS median | 8227.76 | 8146.23 | **8244.15** |
| MiB/s median | 8229.82 | 8149.06 | **8247.07** |
| RPS CV | **0.017** | 0.031 | 0.026 |
| core-ms/req | **0.101514** | 0.107765 | 0.111488 |
| CPU/MiB (core-ms) | **0.101514** | 0.107765 | 0.111488 |
| RSS avg MiB | 50.79 | **49.46** | 55.91 |
| RSS peak MiB | 105.6 | **98.01** | 101.6 |
| P50 ms | **13.91** | 15.50 | 16.58 |
| P95 ms | **19.28** | 21.55 | 24.41 |
| P99 ms | **24.79** | 26.55 | 30.68 |

Sign convention: for RPS/throughput, positive ExyonQ−rival = ExyonQ higher (better).
For CPU/RSS/latency, negative ExyonQ−rival % on cost metrics = ExyonQ cheaper/faster (better).

```text
EXYONQ_RPS_VS_NGINX_PERCENT = +1.00%
EXYONQ_RPS_VS_OLS_PERCENT = -0.20%
EXYONQ_CPU_COST_VS_NGINX_PERCENT = -5.80%
EXYONQ_CPU_COST_VS_OLS_PERCENT = -8.95%
EXYONQ_RSS_VS_NGINX_PERCENT = +2.69%
EXYONQ_RSS_VS_OLS_PERCENT = -9.17%
EXYONQ_P50_VS_NGINX_PERCENT = -10.26%   # lower latency better
EXYONQ_P95_VS_NGINX_PERCENT = -10.53%
EXYONQ_P99_VS_NGINX_PERCENT = -6.63%
EXYONQ_P99_VS_OLS_PERCENT = -19.20%
```

### Cgroup companion

| | user µs/req | system µs/req | total |
|--|-------------|---------------|-------|
| ExyonQ | **7.583** | **150.010** | **157.593** |
| NGINX | 15.447 | 158.159 | 173.605 |
| OLS | 21.725 | 154.491 | 176.216 |

### Reliability (separated)

```text
EXYONQ_SERVER_ERROR_COUNT = NOT_MEASURED
NGINX_SERVER_ERROR_COUNT = NOT_MEASURED
OLS_SERVER_ERROR_COUNT = NOT_MEASURED
EXYONQ_PROBE_ARTIFACT_COUNT = 0
NGINX_PROBE_ARTIFACT_COUNT = 200   # pct-probe connection closed ONLY
OLS_PROBE_ARTIFACT_COUNT = 0
P3_WINNER_RELIABILITY = NOT_PROVEN
```

### Transfer geometry

```text
SENDFILE_CALLS_PER_REQ = NOT_MEASURED
SPLICE_CALLS_PER_REQ = NOT_MEASURED
TCP_OUTSEGS_PER_MIB = NOT_MEASURED
INSTRUCTIONS_PER_REQ = NOT_MEASURED
CONTEXT_SWITCHES_PER_REQ = NOT_MEASURED
OFF_CPU_FRACTION = NOT_MEASURED
```

## Winners

```text
P3_WINNER_RPS = OLS
P3_WINNER_THROUGHPUT_BYTES = OLS
P3_WINNER_CPU = EXYONQ
P3_WINNER_CPU_PER_MIB = EXYONQ
P3_WINNER_MEMORY = NGINX
P3_WINNER_P50 = EXYONQ
P3_WINNER_P95 = EXYONQ
P3_WINNER_P99 = EXYONQ
P3_WINNER_RELIABILITY = NOT_PROVEN
P3_WINNER_STABILITY = EXYONQ
```

## Competitive admission

```text
P3_TECHNICAL_CORRECTNESS = PASS
P3_ENGINEERING_BASELINE = CLOSED
P3_PERFORMANCE_COMPETITIVENESS = AHEAD
P3_COMPETITIVE_WIN = YES
P3_MIGRATION_VALUE_VS_NGINX = PARTIAL
P3_MIGRATION_VALUE_VS_OLS = PARTIAL
P3_MARKET_READINESS = NOT_READY
```

### Why COMPETITIVE_WIN = YES (Pareto)

ExyonQ materially wins **CPU** and **P50/P95/P99** (and stability) against both rivals,
with RPS within noise of OLS (−0.2%) and ahead of NGINX (+1.0%).
RSS vs NGINX is only ≈+2.7% (not treated as material critical loss);
RSS vs OLS is better.

### Migration answers

```text
WHY_MIGRATE_FROM_NGINX_TO_EXYONQ_ON_P3 =
  Lower CPU/request (−5.8%) and clearly better tail/median latency
  (P50/P95 ≈ −10%, P99 ≈ −6.6%) at similar throughput. Memory not better.
  → PARTIAL migration value (performance package real; not a full ops story).

WHY_MIGRATE_FROM_OLS_TO_EXYONQ_ON_P3 =
  Near-equal RPS with substantially better latency (P99 ≈ −19%) and lower CPU
  (−9%) plus lower RSS. → PARTIAL (strong static-file latency/efficiency case;
  still not a complete product migration narrative alone).
```

## Case P3-C1

```text
NEXT_CAUSAL_MEASUREMENT_PROPOSAL = NONE
NEXT_PRODUCT_WIP_PROPOSAL = NONE
P4_STARTED = NO
```

Do **not** start P4 or product repair from this WIP — owner review.

## P1/P2 axes preserved

```text
P1_COMPETITIVE_WIN = NOT_PROVEN
P2_COMPETITIVE_WIN = NOT_PROVEN
P2_RELIABILITY_COMPARISON = NOT_PROVEN_DUE_TO_PROBE_ARTIFACT
P2_WINNER_RELIABILITY = NOT_PROVEN
```
