# V044 P2 — Static 64 KiB competitive admission baseline

```text
STATUS = COMPLETE
CASE = P2-C2
WIP = V044_P2_STATIC_64K_COMPETITIVE_ADMISSION_BASELINE
MODE = COMPETITIVE_MEASUREMENT_ONLY
PRODUCT_MUTATION = NO
PRODUCT_CODE_CHANGED = NO
PERFORMANCE_REPAIR = NOT_AUTHORIZED
P3_STARTED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

PRODUCT_AUTHORITY_HEAD = 06742e1bdc67b5ed86ef95750399e9edad16d646
PRODUCT_AUTHORITY_TREE = 5818095793373bd322ac13a2b3ebcf32661941e1
PRODUCT_BINARY_SHA256 = 09301dc867a4161ebf1da24635f97df14acffcc25f8211b1e475b97b6defe36f

EVIDENCE_NETCUP =
  /root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p2-static-64k-3way-20260821T212806Z/
LAB_COPY =
  .exyonq-local/evidence/v044-p2-static-64k-3way-20260821T212806Z/
HARNESS = scripts/remote/v044-p2-static-64k-live-3way.sh
```

## Contract reconciliation

```text
P2_CONTRACT_RECONCILIATION_REQUIRED = NO
P2_CONTRACT_SOURCE =
  benchmarks/scenarios/perf/scenarios.toml [p2]
  + P1 live-3way competitive meter (same as seal harness)
P2_PAYLOAD_BYTES = 65536
P2_PROTOCOL = HTTP/1.1
P2_KEEPALIVE = rewrk connection reuse
P2_CONNECTIONS = 100
P2_LOAD_THREADS = 2
P2_WARMUP_SECONDS = 20
P2_MEASURE_SECONDS = 30
P2_REPETITIONS = 5
P2_CPUSET = 0-7
P2_CPU_QUOTA = 0
P2_WAF_MODE = explicit enabled=false mode=disabled (RAW)
P2_EXECUTION_MODEL = Docker compose v044p1auth-clean + p1-authoritative overlay
P2_PATH = /site/64k.bin
P2_COMPARABILITY = PASS
```

Ambient dirty lab tree was **not** used to build the measured binary.

## Correctness sanity

```text
GET_64K_STATUS = 200
GET_64K_BYTES = 65536
GET_64K_HASH_MATCH = YES
  hash = 1f8745f0d2d1387ec1af2211a3cf417b2e9e885e853472649c1d979d0e9370e3
  (ExyonQ = NGINX = OLS)
KEEPALIVE_64K = PASS  # serial 200×65536 twice each
HEAD_64K = PASS       # 200 bodyless for all three
```

## CAUSE3C / geometry observation (not reopened)

```text
UID_ACCOUNTING_ASYMMETRY = YES
  ExyonQ process uid=10001(exyonq)
  NGINX workers run as nginx; OLS workers as nobody
CAUSE3C_ASYMMETRY_PRESENT = YES  # UID present; not proven causal this run
SPLICE_PIPE_GEOMETRY_ANOMALY = NO
PIPE_GEOMETRY = pipe-max-size=1048576 (exyonq ns)
```

Historical CAUSE3C / splice_pipe soft budget are **context only** — not current failure evidence.

## Authoritative three-way (docker-stats meter)

| | ExyonQ | NGINX | OLS |
|--|--|--|--|
| RPS median | **97931.70** | 96136.12 | **101594.97** |
| RPS CV | 0.090 | **0.041** | **0.037** |
| core-ms/req | **0.019339** | 0.020473 | 0.020591 |
| RSS avg MiB | 13.70 | **11.56** | 51.72 |
| RSS peak MiB | 16.64 | **14.61** | 54.56 |
| P50 ms | 1.28 | **1.24** | 1.29 |
| P95 ms | 1.94 | **1.90** | 2.02 |
| P99 ms | 2.49 | **2.41** | 2.54 |

```text
EXYONQ_RPS_VS_NGINX_PERCENT = +1.87%   # higher better
EXYONQ_RPS_VS_OLS_PERCENT = -3.61%
EXYONQ_CPU_COST_VS_NGINX_PERCENT = -5.54%  # lower cost better
EXYONQ_CPU_COST_VS_OLS_PERCENT = -6.08%
EXYONQ_RSS_VS_NGINX_PERCENT = +18.45%
EXYONQ_RSS_VS_OLS_PERCENT = -73.51%
```

### Cgroup companion (separate meter)

| | user µs/req | system µs/req | total |
|--|-------------|---------------|-------|
| ExyonQ | **3.579** | **22.884** | **26.464** |
| NGINX | 4.344 | 25.146 | 29.490 |
| OLS | 5.663 | 27.873 | 33.537 |

Direction agrees: ExyonQ cheapest CPU/request on both meters.

### Reliability (percentile probe window)

```text
EXYONQ_PROBE_ARTIFACT_COUNT = 0
NGINX_PROBE_ARTIFACT_COUNT = 3000   # "connection closed" on pct probe ONLY
OLS_PROBE_ARTIFACT_COUNT = 200
EXYONQ_SERVER_ERROR_COUNT = NOT_MEASURED
NGINX_SERVER_ERROR_COUNT = NOT_MEASURED
OLS_SERVER_ERROR_COUNT = NOT_MEASURED
P2_RELIABILITY_COMPARISON = NOT_PROVEN_DUE_TO_PROBE_ARTIFACT
P2_WINNER_RELIABILITY = NOT_PROVEN
```

Do **not** treat pct-probe connection-closed counts as proven server-side request failures.
Series JSON paths remain primary for RPS.

### Transfer geometry

```text
BYTES_PER_REQUEST = 65536
SENDFILE_CALLS_PER_REQ = NOT_MEASURED
WRITEV_CALLS_PER_REQ = NOT_MEASURED
WRITE_CALLS_PER_REQ = NOT_MEASURED
SPLICE_CALLS_PER_REQ = NOT_MEASURED
TCP_OUTSEGS_PER_MIB = NOT_MEASURED
INSTRUCTIONS_PER_REQ = NOT_MEASURED
BRANCHES_PER_REQ = NOT_MEASURED
CYCLES_PER_REQ = NOT_MEASURED
CONTEXT_SWITCHES_PER_REQ = NOT_MEASURED
OFF_CPU_FRACTION = NOT_MEASURED
PSS = NOT_MEASURED  # this baseline (docker RSS used)
```

## Winners

```text
P2_WINNER_RPS = OLS
P2_WINNER_CPU = EXYONQ
P2_WINNER_MEMORY = NGINX
P2_WINNER_P50 = NGINX
P2_WINNER_P95 = NGINX
P2_WINNER_P99 = NGINX
P2_WINNER_RELIABILITY = NOT_PROVEN
P2_RELIABILITY_COMPARISON = NOT_PROVEN_DUE_TO_PROBE_ARTIFACT
P2_WINNER_STABILITY = OLS
```

## Independent state axes

```text
TECHNICAL_CORRECTNESS = PASS
ENGINEERING_BASELINE = CLOSED
PERFORMANCE_COMPETITIVENESS = MIXED
COMPETITIVE_WIN = NOT_PROVEN
MIGRATION_VALUE_VS_NGINX = NOT_PROVEN
MIGRATION_VALUE_VS_OLS = NOT_PROVEN
MARKET_READINESS = NOT_READY
```

### Migration answers

```text
WHY_MIGRATE_FROM_NGINX_TO_EXYONQ_ON_P2 =
  Slight RPS edge (+1.9%) and better CPU/request (−5.5%), but loses RSS (~+18%)
  and P50/P95/P99 by small margins. Not a compelling migration package.

WHY_MIGRATE_FROM_OLS_TO_EXYONQ_ON_P2 =
  Much lower RSS and better CPU/request and slightly better P99, but loses RPS
  (−3.6%). “Better efficiency, lower throughput” is not proven migration value.
```

## Case P2-C2

Technically sound baseline; competitively **not won**.

```text
TOP_OBSERVED_DEFICITS =
  1. RPS vs OLS (−3.6%)
  2. RSS vs NGINX (+18%)
  3. Latency P50/P95/P99 vs NGINX (small; ~2–3%)
  4. RPS CV higher than rivals (0.090 vs ~0.04)

NEXT_CAUSAL_MEASUREMENT_PROPOSAL = NONE
NEXT_PRODUCT_WIP_PROPOSAL = NONE
```

No single high-value causal root is directly proven from this baseline alone.
Do **not** start P3 or product repair from this WIP.

## P1 axes preserved

```text
P1_ENGINEERING_BASELINE = CLOSED
P1_COMPETITIVE_WIN = NOT_PROVEN
P1_MARKET_READINESS = NOT_READY
P1_PRODUCT_REPAIR = NOT_AUTHORIZED
```
