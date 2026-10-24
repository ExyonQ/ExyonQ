# V044 P4 — Proxy 1 KiB competitive admission baseline

```text
STATUS = COMPLETE
CASE = P4-C2
WIP = V044_P4_PROXY_1K_COMPETITIVE_ADMISSION_BASELINE
MODE = READ_ONLY_COMPETITIVE_BASELINE
PRODUCT_MUTATION = NO
PRODUCT_CODE_CHANGED = NO
PERFORMANCE_OPTIMIZATION = NOT_AUTHORIZED
HARNESS_ONLY_FIX = YES
P5_STARTED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN

PRODUCT_AUTHORITY_HEAD = ade997a362e60d1dc6eafc984da454930600845a
PRODUCT_AUTHORITY_TREE = 4ae6ffc7528d0881be23fcac246c345dcb9a1606
PRODUCT_BINARY_SHA256 = e28c0abe18587f2d911d492be64cbec42372383e011417a6c578be52da5ba459
DOCS_HEAD = eb689ebe2b4cdf7dd639f7d2c0c99fc63b117a5f

RUN_112732Z = INVALID_NON_AUTHORITATIVE
RUN_115113Z = AUTHORITATIVE_VALID_P4_BASELINE

EVIDENCE_NETCUP =
  /root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p4-proxy-1k-3way-20260822T115113Z/
HARNESS = scripts/remote/v044-p4-proxy-1k-competitive-baseline.sh
SCOREBOARD = docs/benchmarks/v044-p1-p14-competitive-dashboard.json
```

## 1. Exact P4 contract

```text
P4_PROFILE = PROXY_1K
P4_CONTRACT_SOURCE =
  benchmarks/scenarios/perf/scenarios.toml [p4]
  + xtask/fixtures/scenarios.toml [p4]
  + bench docker configs (v044p1auth-clean overlay)
P4_CONTRACT_RECONCILIATION_REQUIRED = NO
P4_REQUEST_PATH = /api/
P4_PAYLOAD_BYTES = 1024
P4_PROTOCOL_CLIENT_SIDE = HTTP/1.1
P4_PROTOCOL_UPSTREAM_SIDE = HTTP/1.1
P4_UPSTREAM_TYPE = exyonq-upstream (real HTTP/1.1 TCP peer, tools/upstream/)
P4_UPSTREAM_LISTEN = upstream:9000
P4_KEEPALIVE_CLIENT = rewrk default (connection reuse)
P4_KEEPALIVE_UPSTREAM = enabled (upstream Connection: keep-alive; nginx/ols persistConn)
P4_CONNECTIONS = 100
P4_LOAD_THREADS = 2
P4_WARMUP_SECONDS = 20
P4_MEASURE_SECONDS = 30
P4_REPETITIONS = 5
P4_CPUSET = 0-7
P4_CPU_QUOTA = 0
P4_EXECUTION_MODEL = Docker compose project=v044p1auth-clean overlay=p1-authoritative
P4_BENCHMARK_CATEGORY = RAW_TRANSPORT
P4_BENCHMARK_CATEGORY_NOTE =
  ExyonQ bench.toml explicit WAF off + access/otel off;
  nginx access_log off;
  comparable minimal proxy path
```

## 2. Source / product authority

```text
PRODUCT_COMMIT = ade997a362e60d1dc6eafc984da454930600845a
PRODUCT_TREE = 4ae6ffc7528d0881be23fcac246c345dcb9a1606
PRODUCT_BINARY_SHA256 = e28c0abe18587f2d911d492be64cbec42372383e011417a6c578be52da5ba459
UPSTREAM_BINARY_SHA256 = 1881808573841c64475ecc403c58c7f76a747db581d30e3abd72a34918a6484e
DOCKER_IMAGE = v044p1-geom8-exyonq
COMPOSE_PROJECT = v044p1auth-clean
WORKER_GEOMETRY = 8/8/8 (env unset; product defaults on 8-CPU cpuset)
HOST = Netcup amd64 (Linux evidence authority)
```

## 3. Benchmark category

```text
BENCHMARK_CATEGORY = RAW_TRANSPORT
PRODUCT_SEMANTIC_SHORTCUTS = NO
BENCH_API_CACHE_PATHS = NOT_USED
WAF = disabled
ACCESS_LOG = off (all comparators)
```

## 4. Comparator matrix

| Comparator | Role | Image / config |
|------------|------|----------------|
| ExyonQ | subject | `v044p1-geom8-exyonq` + p1-authoritative overlay |
| NGINX-stable | rival | `benchmarks/configs/nginx/nginx.conf` (bench harness) |
| OLS-latest | rival | OpenLiteSpeed bench docker config |
| exyonq-upstream | real upstream peer | `:9000`, body from `1k.bin` (1024 B) |

Same upstream, payload, Docker internal network, load (`rewrk -c 100 -t 2`), warmup/measure/reps.

## 5. Real upstream details

```text
UPSTREAM_CONTAINER = v044p1auth-clean-upstream-1
UPSTREAM_URL_DIRECT = http://upstream:9000/api/
UPSTREAM_BODY_FILE = 1k.bin
UPSTREAM_BODY_BYTES = 1024
UPSTREAM_BODY_SHA256 = 49abd65bbf7f7e40c7055093ed2e3fd75f2f602f2c5fcf955c213e3135eb03f7
UPSTREAM_PROTOCOL = HTTP/1.1
```

## 6. Upstream ceiling

Direct load to upstream (no proxy), same load generator geometry:

```text
UPSTREAM_DIRECT_RPS = 150720.16
UPSTREAM_DIRECT_P50_MS = 0.91
UPSTREAM_DIRECT_P95_MS = 1.6
UPSTREAM_DIRECT_P99_MS = 2.24
UPSTREAM_TO_EXYONQ_HEADROOM_RATIO = 2.06
UPSTREAM_BOTTLENECK = NO
```

Proxy RPS (~73k ExyonQ) is well below upstream ceiling — deficit is not upstream-limited.

## 7. Correctness gate

```text
CORRECTNESS_GATE = PASS
BODY_HASH_MATCH = YES
exyonq_proxy = 200 1024 hash match
nginx_proxy = 200 1024 hash match
ols_proxy = 200 1024 hash match
upstream_direct = 200 1024 hash match
```

Artifacts: `sanity/correctness.txt`, `sanity/proxy_hash.txt`, `sanity/upstream_body.sha`.

## 8. Invalid run `112732Z` history

```text
INVALID_RUN = v044-p4-proxy-1k-3way-20260822T112732Z
INVALID_RUN_CLASSIFICATION = INVALID_NON_AUTHORITATIVE
INVALIDATION_REASON = HARNESS_DEFECT_NGINX_UPSTREAM_NO_KEEPALIVE
```

Observed symptom (invalid run only — **do not cite as competitive truth**):

- NGINX RPS collapsed to ~500–600 range vs ExyonQ ~68k–71k and OLS ~99k–103k under identical harness.
- NGINX error pattern consistent with ephemeral port exhaustion (`connect() failed (99: Address not available)`).

Marked: `reports/invalid.txt` with `HARNESS_DEFECT=nginx_upstream_no_keepalive`.

Intermediate incomplete runs (`114644Z`, `114827Z`, `114956Z`) are **not** authority — superseded by `115113Z`.

## 9. Exact harness defect

`benchmarks/configs/nginx/nginx.conf` used bare:

```nginx
proxy_pass http://upstream:9000/;
```

without an upstream keepalive pool. Under `rewrk -c 100`, NGINX opened a new TCP connection per request to upstream → port exhaustion and artificially collapsed NGINX RPS.

## 10. Exact harness-only fix

**File:** `benchmarks/configs/nginx/nginx.conf`

```nginx
upstream bench_upstream {
    server upstream:9000;
    keepalive 512;
    keepalive_requests 1000000;
    keepalive_timeout 60s;
}
location /api/ {
    proxy_http_version 1.1;
    proxy_set_header Connection "";
    proxy_pass http://bench_upstream/;
    ...
}
```

**File:** `scripts/remote/v044-p4-proxy-1k-competitive-baseline.sh`

- Added `proxy_comparability_smoke()` — fails if NGINX RPS < 25% of min(ExyonQ, OLS).
- Fixed `timeout` placement: `compose exec -T bench-runner timeout … rewrk` (not `timeout … compose`).

**PRODUCT_MUTATION = NO** — harness/config comparator only.

## 11. Why `112732Z` is forbidden as evidence

NGINX numbers from `112732Z` reflect a **harness misconfiguration**, not NGINX proxy capability. Using them would:

- falsely imply ExyonQ leads RPS by ~100× on P4;
- violate benchmark comparability (rivals not on equal upstream keepalive semantics);
- contaminate migration-value and competitive-win classification.

**Forbidden for:** scoreboard, dashboard, migration claims, repair prioritization.

## 12. Valid `115113Z` evidence authority

```text
VALID_RUN = v044-p4-proxy-1k-3way-20260822T115113Z
P4_VALIDITY = PASS
P4_COMPARABILITY = PASS
reports/done.txt = present
summary.json = present
15 run JSONs (5 reps × 3 servers) + upstream ceiling + cgroup companion
```

Post-fix smoke (30s): ExyonQ ~77k, NGINX ~95k, OLS ~101k — credible comparability (`sanity/proxy_comparability_smoke.txt`).

## 13. Repetitions (RPS)

| Rep | ExyonQ | NGINX | OLS |
|-----|--------|-------|-----|
| 1 | 73711.24 | 100402.05 | 104007.60 |
| 2 | 74591.91 | 83608.45 | 93208.40 |
| 3 | 69739.46 | 96446.70 | 99986.40 |
| 4 | 72056.81 | 98854.00 | 90410.55 |
| 5 | 73096.77 | 94096.32 | 97310.38 |

## 14. Medians

| | ExyonQ | NGINX | OLS |
|--|--------|-------|-----|
| RPS median | **73096.77** | 96446.70 | **97310.38** |

## 15. P50 / P95 / P99 (ms)

| | ExyonQ | NGINX | OLS |
|--|--------|-------|-----|
| P50 | 1.66 | 1.57 | **1.50** |
| P95 | **2.71** | 3.05 | 2.88 |
| P99 | **3.53** | 4.33 | 4.17 |

## 16. CPU metrics

Meter: docker-stats derived **core-ms/req** (same-session geom8 harness).

| | core-ms/req | user µs/req | system µs/req |
|--|-------------|-------------|---------------|
| ExyonQ | 0.2066 | 21.90 | 18.68 |
| NGINX | 0.1482 | 6.78 | 21.72 |
| OLS | **0.1319** | 7.72 | 16.33 |

```text
EXYONQ_CPU_VS_NGINX_PERCENT = +39.4%   # higher cost = worse
EXYONQ_CPU_VS_OLS_PERCENT = +56.7%
```

## 17. RSS metrics (MiB, docker stats avg)

| | avg | peak |
|--|-----|------|
| ExyonQ | 33.18 | 35.38 |
| NGINX | **9.95** | 11.18 |
| OLS | 16.31 | 16.88 |

## 18. CV / stability

| | RPS CV |
|--|--------|
| ExyonQ | **0.0257** (winner stability) |
| NGINX | 0.0701 |
| OLS | 0.0555 |

```text
P4_WINNER_STABILITY = EXYONQ
```

## 19. Errors

```text
SERIES_SERVER_ERRORS = NOT_MEASURED
PROBE_ARTIFACT_CONNECTION_CLOSED:
  exyonq = 0
  nginx = 2668   # pct-probe artifact only
  ols = 203      # pct-probe artifact only
P4_WINNER_RELIABILITY = NOT_PROVEN
```

## 20. Probe artifacts

Separated from series medians per P2/P3 policy:

- `pct/exyonq.txt`, `pct/nginx.txt`, `pct/ols.txt` — `--pct` probe runs
- NGINX/OLS connection-closed counts are **probe-tool artifacts**, not elevated to SERVER_REQUEST_FAILURES

## 21. Upstream connection reuse metrics

```text
UPSTREAM_CONNECTION_REUSE_METRICS = NOT_MEASURED
KEEPALIVE_SERIAL_PROBE = PASS (sanity/keepalive_serial.txt — 200 1024 × 3 serial)
```

Qualitative: upstream keepalive enabled on all proxies post-fix; direct upstream ceiling confirms peer capacity.

## 22. Winner per metric

| Metric | Winner |
|--------|--------|
| RPS | OLS |
| CPU/request | OLS |
| RSS | NGINX |
| P50 | OLS |
| P95 | ExyonQ |
| P99 | ExyonQ |
| Stability (RPS CV) | ExyonQ |
| Reliability | NOT_PROVEN |

## 23. ExyonQ vs NGINX deltas

```text
EXYONQ_VS_NGINX_RPS_PERCENT = -24.21%
EXYONQ_VS_NGINX_P95_PERCENT = -11.1%   # lower latency = better
EXYONQ_VS_NGINX_P99_PERCENT = -18.5%
EXYONQ_VS_NGINX_CPU_PERCENT = +39.4%    # higher cost = worse
EXYONQ_VS_NGINX_RSS_PERCENT = +233%   # material higher RSS
```

## 24. ExyonQ vs OLS deltas

```text
EXYONQ_VS_OLS_RPS_PERCENT = -24.88%
EXYONQ_VS_OLS_P95_PERCENT = -5.9%
EXYONQ_VS_OLS_P99_PERCENT = -15.3%
EXYONQ_VS_OLS_CPU_PERCENT = +56.7%
EXYONQ_VS_OLS_RSS_PERCENT = +103%
```

## 25. Competitive classification

```text
P4_TECHNICAL_CORRECTNESS = PASS
P4_ENGINEERING_BASELINE = CLOSED
P4_PERFORMANCE_COMPETITIVENESS = MIXED
P4_COMPETITIVE_WIN = NOT_PROVEN
```

Rationale: material ~24–25% RPS deficit vs both rivals; ExyonQ wins tail latency (P95/P99) and stability but loses RPS, CPU, P50, and RSS. Not a Pareto win; not P4-C1.

## 26. Migration-value classification

```text
P4_MIGRATION_VALUE_VS_NGINX = NOT_PROVEN
P4_MIGRATION_VALUE_VS_OLS = NOT_PROVEN
DOES_THIS_RESULT_INCREASE_MIGRATION_VALUE = NO
GLOBAL_MIGRATION_VALUE = NOT_PROVEN
```

Tail latency advantage exists but does **not** compensate ~24–25% throughput loss and higher CPU/RSS.

## 27. Market-readiness classification

```text
P4_MARKET_READINESS = NOT_READY
```

Engineering baseline closed; competitive proxy story not proven for migration.

## 28. Does P4 increase global migration value?

```text
DOES_THIS_RESULT_INCREASE_MIGRATION_VALUE = NO
```

Adds a measured proxy baseline (honest deficit visibility). Does not strengthen migration narrative.

## 29. Next-step decision

```text
CASE = P4-C2
NEXT_CAUSAL_MEASUREMENT_PROPOSAL = COMPLETE
  # v044-p4-proxy-path-causal-profiling-20260825T203611Z — CASE P4P-B
  # doc: docs/governance/v044-p4-proxy-path-causal-profiling.md
CAP034_CONTRACT_REVIEW = COMPLETE
  # docs/governance/v044-cap034-proxy-framing-keepalive-contract-review.md — CASE C34-C
NEXT_MEASUREMENT_WIP_PROPOSAL = V044_CAP034_FRAMING_KEEPALIVE_BOUNDED_VALUE_AB
NEXT_PRODUCT_WIP_PROPOSAL = NONE
NO_MORE_OPTIMIZATION_FOR_BENCHMARK_PRIDE = YES
NO_CODE_WITHOUT_EXPECTED_PRODUCT_VALUE = YES
```

```text
IS_P4_RPS_LOSS_MATERIAL = YES
IS_P4_RPS_LOSS_REPEATABLE = YES   # consistent across 5 reps
IS_THERE_ALREADY_A_STRONG_CAUSAL_ROOT = NO
```

### Migration answers

```text
WHY_MIGRATE_FROM_NGINX_TO_EXYONQ_ON_P4 =
  Tail latency advantage (P95 2.71 vs 3.05 ms; P99 3.53 vs 4.33 ms) and best RPS
  stability (CV 0.026), but ~24% lower RPS, ~39% higher CPU/request, and ~3× RSS.
  → NOT_PROVEN migration value.

WHY_MIGRATE_FROM_OLS_TO_EXYONQ_ON_P4 =
  Tail latency advantage (P95/P99) and stability, but ~25% lower RPS, ~57% higher
  CPU/request, and ~2× RSS. → NOT_PROVEN migration value.
```

### Top observed deficits (descriptive, not causal)

1. ~24–25% RPS vs NGINX and OLS (repeatable across 5 reps)
2. Higher CPU/request vs both rivals
3. Higher RSS vs NGINX (and vs OLS)

## 30. STOP / P5_NOT_STARTED

```text
P5_STARTED = NO
PRODUCT_REPAIR_AUTHORIZED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
PUSH = NO
TAG = NO
RELEASE = NO
```

Do **not** start P5 or product repair from this WIP.
