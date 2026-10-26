# V044 — Benchmark methodology qualification (P1 Static + P4 Proxy Docker-internal)

```text
WIP = V044_BENCHMARK_METHODOLOGY_QUALIFICATION
STATUS = PARTIAL (P1 + P4 Proxy method-qualified; FastCGI blocked)
DATE = 2026-09-05
OFFICIAL_BENCHMARK_AUTHORIZED = NO
OFFICIAL_RANKING = NO
PUBLICATION = NO
PRODUCT_MUTATION = NO
HARNESS_MUTATION = NO
```

## Why this WIP exists

Under an identical P1 Docker-internal manifest, the ExyonQ↔NGINX RPS differential moved across sessions from historical **−12.53%** (docker_stats era) to **−5%** then **+1.3%**, then in a five-session Netcup qualification to **−1.36%** with overlapping bootstrap median CIs.

Conclusion already proven before this WIP: **a single RPS delta is not enough** without quantified session/host variance, and historical **`docker stats` CPU/RSS must not be used for comparative ranking**.

A follow-on extension (same protocol) qualifies **P4 Proxy** (`/api/` → real `exyonq-upstream`) on both evidence hosts. **FastCGI three-way competitive methodology remains blocked**: no canonical Docker-internal ExyonQ+NGINX+OLS FastCGI harness exists; inventing one was forbidden.

## Scope

| In scope | Out of scope |
|----------|----------------|
| P1 Static `/site/1k.bin` Docker-internal | FastCGI three-way (blocked — no canonical harness) |
| P4 Proxy `/api/` → real HTTP upstream | Official competitive campaign |
| ExyonQ + NGINX + OpenLiteSpeed | Cross-arch aggregation / marketing |
| Netcup amd64 + Oracle arm64 | Product repair / tuning / TLS / HTTP/3 |

## Protocol (qualified where executed)

### Session independence

- Recreate / reuse the historical compose project `v044p1auth-clean` with overlay `docker-compose.p1-authoritative.yml`.
- **Five independent sessions per host**, each with one measured pass per contender.
- Warm-up **20 s** separated from measure **30 s** (`rewrk -c 100 -t 2`).
- Balanced rotation:

```text
S1: ExyonQ → NGINX → OpenLiteSpeed
S2: NGINX → OpenLiteSpeed → ExyonQ
S3: OpenLiteSpeed → ExyonQ → NGINX
S4: ExyonQ → OpenLiteSpeed → NGINX
S5: NGINX → ExyonQ → OpenLiteSpeed
```

- Do not discard replicas. Infrastructure failures require a full session redo with both traces kept.

### Manifest fields (per session)

```text
HOST / ARCH / KERNEL / UTC_TIMESTAMP
EXYONQ_BINARY_SHA256
CONTAINER_IMAGE_DIGESTS / SERVER identity
DOCROOT_SHA256 (1k.bin) / UPSTREAM_PAYLOAD_SHA256 (proxy)
CONFIG_SHA256 (exyonq bench.toml, nginx.conf)
UPSTREAM_BINARY_SHA256 + health probes (proxy)
CPUSET / CPU_QUOTA / MEMORY_LIMIT
DOCKER_NETWORK
LOAD_GENERATOR image + command
WARMUP_SECONDS / MEASURE_SECONDS
BACKGROUND_LOAD_SNAPSHOT
```

### Mandatory measure-window sources

| Metric | Authority | Forbidden as ranking authority |
|--------|-----------|--------------------------------|
| RPS / latency / bytes | `rewrk --json` measure window | — |
| CPU | cgroup `cpu.stat` `usage_usec` delta → CPU-seconds and CPU-seconds/request | `docker stats` CPU% |
| RSS | sum of `/proc/<pid>/status` `VmRSS` over `cgroup.procs` | `docker stats` MemUsage |
| Memory cgroup | `memory.current` / `memory.peak` recorded **separately** | conflating with RSS |
| Isolation | main PID + full `cgroup.procs` before/after | host-wide / generator CPU |

`docker stats` may be retained only as **AUXILIARY** historical continuity.

### Statistical reporting (per host, never cross-arch)

For each contender: median, mean, min, max, stdev, CV, IQR, and a **bootstrap 95% CI of the median** (documented seed + method).  
Order effect is analyzed as mean RPS by rotation position.  
**No invented significance threshold** for “winner”: report whether median CIs overlap for that host/scenario.

## Identity notes

### Netcup amd64

```text
EXYONQ_BINARY_SHA256 = 09301dc867a4161ebf1da24635f97df14acffcc25f8211b1e475b97b6defe36f
IMAGE = v044p1auth-exyonq (historical seal)
CPUSET = 0-7 (8 cores)
REQUIRE_EXACT_SHA = YES → PASS
```

### Oracle arm64

```text
EXYONQ_BINARY_SHA256 = 8c7b33f84fdfedab3f56c6a393238f18012130f96394ea973042f7d2469c86f0
SOURCE = native arm64 docker build from transferred seal tree /opt/exyonq-v044-p1-seal-06742e1b
CPUSET = 0-3 (4 cores)
REQUIRE_EXACT_SHA = NO (amd64 seal digest cannot exist on arm64)
IDENTITY_CLASS = SAME_SEAL_TREE_NATIVE_ARCH_BINARY
```

Absolute RPS must **not** be compared across Netcup↔Oracle (different core counts and binaries).

---

## P1 Static — results (methodology, not ranking)

### Evidence paths (P1)

| Host | Remote | Local fetch |
|------|--------|-------------|
| Netcup | `/tmp/exyonq-p1-methodqual-netcup-20260905T074622Z/` | `<workspace>/temp/exyonq-p1-methodqual-20260905/fetched/netcup/` |
| Oracle | `/tmp/exyonq-p1-methodqual-oracle-20260905T075946Z/` | `<workspace>/temp/exyonq-p1-methodqual-20260905/fetched/oracle/` |

Lab context HEAD at P1 qualification time: `5e5895f58e056e6b73a82c732245f3241b4c17fa` / tree `f81dcb95a7788610014bd0a57a76966b86e5a476` (product binary authority remains the seal image / seal-tree build — not a rebuild of that HEAD).

### Netcup amd64 (n=5)

| Contender | RPS median | CV | Bootstrap median CI95 | cpsr median | RSS sum MiB median |
|-----------|------------|----|-----------------------|-------------|--------------------|
| ExyonQ | 154288 | 0.067 | [149243, 174627] | 2.17e-5 | 24.97 |
| NGINX | 156417 | 0.032 | [149839, 162621] | 2.96e-5 | 32.26 |
| OLS | 143367 | 0.027 | [139912, 149618] | 2.07e-5 | 71.08 |

```text
EXYONQ_VS_NGINX_MEDIAN_PCT ≈ −1.36%
BOOTSTRAP_MEDIAN_CI_OVERLAP = YES
→ cannot claim stable RPS separation on this host/scenario from these 5 sessions
ORDER_EFFECT_MEAN_RPS_BY_POS ≈ {0: 155275, 1: 152244, 2: 153056} (mild)
THROTTLING = nr_throttled=0 observed
```

### Oracle arm64 (n=5)

| Contender | RPS median | CV | Bootstrap median CI95 | cpsr median | RSS sum MiB median |
|-----------|------------|----|-----------------------|-------------|--------------------|
| ExyonQ | 82542 | 0.005 | [81990, 82969] | 3.21e-5 | 18.36 |
| NGINX | 73257 | 0.018 | [71861, 75067] | 3.55e-5 | 18.15 |
| OLS | 86198 | 0.032 | [80515, 87157] | 2.89e-5 | 42.16 |

```text
EXYONQ_VS_NGINX_MEDIAN_PCT ≈ +12.67% on this host only
BOOTSTRAP_MEDIAN_CI_OVERLAP = NO on this host only
→ host-local non-overlap is NOT an official ranking and must not be pooled with Netcup
THROTTLING = nr_throttled=0 observed
```

---

## P4 Proxy — results (methodology, not ranking)

### Scenario identity

```text
SCENARIO_ID = P4_PROXY_1K
PATH = /api/
UPSTREAM = real exyonq-upstream (HTTP :9000) inside compose project v044p1auth-clean
PAYLOAD_SHA256 = 49abd65bbf7f7e40c7055093ed2e3fd75f2f602f2c5fcf955c213e3135eb03f7  (= 1k.bin)
EXYONQ_BENCH_TOML_SHA256 = 1724545863145a7bffd0fc5b92872d629bf65b4fa811cd03e7add6a1f770bf79
NGINX_CONF_SHA256 = 5ec09e8918f447d07d21a4806881a3ea16d137ce869402cc774fd24ac9f994c1
CANONICAL_HARNESS_REFERENCE = scripts/remote/v044-p4-proxy-1k-competitive-baseline.sh
PROTOCOL = same 5×3 rotation + PID/cgroup isolation as P1 (external observer; no product/harness mutation)
```

Upstream binary digests differ by architecture (as expected for native builds):

```text
NETCUP_UPSTREAM_BIN_SHA256 = 1881808573841c64475ecc403c58c7f76a747db581d30e3abd72a34918a6484e
ORACLE_UPSTREAM_BIN_SHA256 = c5e08be973f856eb8ffd156519adaef359e7804d037b5e64031c91f1331cb033
UPSTREAM_OR_PHP_FPM_HEALTH = PASS (200/1024 probes on all three reverse proxies + upstream)
```

### Evidence paths (P4)

| Host | Remote | Local fetch |
|------|--------|-------------|
| Netcup | `/tmp/exyonq-p4-methodqual-netcup-20260905T085821Z/` | `<workspace>/temp/exyonq-p1-methodqual-20260905/fetched/proxy-netcup/` |
| Oracle | `/tmp/exyonq-p4-methodqual-oracle-20260905T085832Z/` | `<workspace>/temp/exyonq-p1-methodqual-20260905/fetched/proxy-oracle/` |

Lab entry HEAD for this Proxy extension: `791d71cef6bd4fd23dceca4532117dbebef2b101` / tree `5a03c51d89d11402cb24f8f9ff716dfd55407201`. Binary authority remains the seal image / seal-tree build.

### Netcup amd64 (n=5)

| Contender | RPS median | CV | Bootstrap median CI95 | cpsr median | RSS sum MiB median |
|-----------|------------|----|-----------------------|-------------|--------------------|
| ExyonQ | 89253 | 0.040 | [85630, 94603] | 4.55e-5 | 35.84 |
| NGINX | 104730 | 0.036 | [97675, 107581] | 3.65e-5 | 33.57 |
| OLS | 109125 | 0.021 | [104945, 111185] | 3.17e-5 | 74.59 |

```text
EXYONQ_VS_NGINX_MEDIAN_PCT ≈ −14.78%
BOOTSTRAP_MEDIAN_CI_OVERLAP_EXYONQ_NGINX = NO
BOOTSTRAP_MEDIAN_CI_OVERLAP_NGINX_OLS = YES
ORDER_EFFECT_MEAN_RPS_BY_POS ≈ {0: 98126, 1: 101373, 2: 102886}
→ host-local non-overlap ExyonQ↔NGINX is methodology signal only — NOT official ranking
CPU/RSS = PID/cgroup authority; docker stats AUXILIARY_ONLY
```

### Oracle arm64 (n=5)

| Contender | RPS median | CV | Bootstrap median CI95 | cpsr median | RSS sum MiB median |
|-----------|------------|----|-----------------------|-------------|--------------------|
| ExyonQ | 18634 | 0.017 | [18253, 19021] | 1.20e-4 | 33.50 |
| NGINX | 27545 | 0.009 | [27099, 27719] | 6.30e-5 | 18.85 |
| OLS | 27580 | 0.021 | [26590, 27846] | 6.24e-5 | 43.58 |

```text
EXYONQ_VS_NGINX_MEDIAN_PCT ≈ −32.35% on this host only
BOOTSTRAP_MEDIAN_CI_OVERLAP_EXYONQ_NGINX = NO
BOOTSTRAP_MEDIAN_CI_OVERLAP_NGINX_OLS = YES
ORDER_EFFECT_MEAN_RPS_BY_POS ≈ {0: 23933, 1: 23935, 2: 25696}
→ must not be pooled with Netcup; absolute RPS not cross-arch comparable
```

### P4 Proxy qualification checklist

```text
MANIFEST_EQUALITY = PASS (all five sessions both hosts; Oracle SHA relaxed per arch)
RAW_RECONCILIATION = PASS
WARMUP_CONTAMINATION = NO
PID_CGROUP_CPU_ISOLATION = PASS
PID_RSS_ISOLATION = PASS
GENERATOR_SATURATION = NO (no loadgen-ceiling claim)
UPSTREAM_OR_PHP_FPM_HEALTH = PASS
HOST_THROTTLING_OR_CONTAMINATION = NO (nr_throttled=0; Netcup hosts co-resident unrelated containers — recorded in background snapshots, not used to void isolation metrics)
ORDER_EFFECT = ANALYZED
SESSION_VARIANCE = QUANTIFIED
PROXY_METHODOLOGY_QUALIFIED = YES (per-host; NOT official ranking)
```

---

## FastCGI — blocked

```text
SCENARIO = BLOCKED_MISSING_CANONICAL_HARNESS
FASTCGI_NETCUP_METHODOLOGY_STATUS = BLOCKED_MISSING_CANONICAL_HARNESS
FASTCGI_ORACLE_METHODOLOGY_STATUS = BLOCKED_MISSING_CANONICAL_HARNESS
```

E2E/soak/reality PHP-FPM scripts exist in-tree, but there is **no** Docker-internal competitive FastCGI methodology harness for ExyonQ + NGINX + OpenLiteSpeed under the same P1/P4 protocol. Per owner constraints, no new harness was designed or invented in this WIP. FastCGI methodology remains **unqualified**.

---

## Qualification gate checklist (aggregate)

```text
P1_STATIC_METHODOLOGY_QUALIFIED = YES
P4_PROXY_METHODOLOGY_QUALIFIED = YES
FASTCGI_METHODOLOGY_QUALIFIED = NO (blocked)
METHODOLOGY_QUALIFIED = PARTIAL
OFFICIAL_BENCHMARK_AUTHORIZED = NO
```

## What is / is not qualified

**Qualified**

- P1 Static Docker-internal measurement protocol with PID/cgroup isolation.
- P4 Proxy Docker-internal measurement protocol with real upstream + same isolation/stats rules.
- Rejection of `docker stats` for CPU/RSS ranking.
- Per-host variance reporting with bootstrap median CI.
- Dual-host protocol execution (Netcup exact seal binary; Oracle native seal-tree binary).

**Not qualified / not authorized**

- Official competitive ranking or publication.
- Cross-architecture RPS comparison.
- Product defect claims from a single residual or from Proxy deltas alone.
- FastCGI methodology (canonical three-way harness missing).

## Failed sessions / anomalies

None in the five Netcup or five Oracle sessions for P1 or P4 Proxy. Percentile tables from `rewrk --pct` (when captured) are **aux** and **not** same-window as RPS.

## Next authorization required

Owner authorization is required before:

1. Designing/adopting a **canonical** FastCGI three-way Docker-internal harness (then re-run methodology qualification).
2. Any official `bench compare`, competitive campaign, or public ranking — even for P1/P4.
3. Any product repair or tuning motivated by Proxy host-local non-overlap.
