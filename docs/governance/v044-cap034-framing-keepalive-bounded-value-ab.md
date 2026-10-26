# V044 — Cap034 framing / keep-alive bounded-value A/B

```text
WIP = V044_CAP034_FRAMING_KEEPALIVE_BOUNDED_VALUE_AB
STATUS = COMPLETE
CASE = AB-C34-B
NON_PRODUCTION_EXPERIMENT = YES
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
```

Isolated product experiment (A1 arm only) to bound how much of the P4 proxy deficit is recoverable by evolving Cap034 downstream framing and client keep-alive. **Not** production adoption.

## Authority

| Field | Value |
|-------|-------|
| SOURCE_HEAD | `ade997a362e60d1dc6eafc984da454930600845a` |
| SOURCE_TREE | `4ae6ffc7528d0881be23fcac246c345dcb9a1606` |
| SOURCE_BINARY_SHA256 (canonical A0 product) | `e28c0abe18587f2d911d492be64cbec42372383e011417a6c578be52da5ba459` |
| EXPERIMENT_BRANCH | `experiment/cap034-framing-keepalive-ab` |
| EXPERIMENT_WORKTREE | `<repo>/.exyonq-local/worktrees/cap034-ab` |
| EXPERIMENT_COMMIT | not committed (working tree only) |
| EXPERIMENT_BINARY_SHA256 (linux/amd64 image build) | `f7248b1806012168615bbfcf32caed24e22df0f9e19c078cac1130d5e4d9f642` |
| CANONICAL_PRODUCT_CHANGED | NO |
| EXPERIMENT_PRODUCT_CHANGED | YES |
| CARGO_LOCK_CHANGED | NO |

## Arms

| Arm | `EXYONQ_PROXY_WIRE_DOWNSTREAM_FRAMING` | Wire (P4 1 KiB) |
|-----|----------------------------------------|-----------------|
| A0 | `cap034_chunked_close` (default) | `Transfer-Encoding: chunked`, `Connection: close` |
| A1 | `safe_known_length_keepalive` | `Content-Length: 1024`, `Connection: keep-alive`, no `Transfer-Encoding` |

Toggle is a legitimate user-visible env (not benchmark-only). P4 workload contract unchanged (geom8, cpuset 0–7, c=100, t=2, warmup 20s, measure 30s, `/api/` 1 KiB).

## Evidence (Netcup amd64)

```text
EVIDENCE_HOST = netcup-bench (Linux amd64)
EVIDENCE_PATH_REMOTE =
  /root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-cap034-framing-keepalive-ab-20260825T222608Z/
RUN_ID = v044-cap034-framing-keepalive-ab-20260825T222608Z
IMAGE = v044-cap034-ab-exyonq:experiment
MATCHED_ALTERNATING = A0,A1 × 5 rounds
```

## Correctness / security (A1)

| Gate | Result |
|------|--------|
| Cap034 routing E2E (`cap-034-routing-host-proxy-e2e.py`, A1 env) | PASS 29/29 |
| LA-001 host switch on same TCP (`wire_keepalive_host_switch_no_frozen_cluster`) | PASS (`response_on_reuse`) |
| Wire probe A0 | chunked + close |
| Wire probe A1 | CL 1024 + keep-alive |
| REQUEST_SMUGGLING / CL+TE / hop-by-hop | covered by existing wire safe + E2E |
| perf stat in container | NOT_AVAILABLE (`perf` absent in image) |

## Performance (matched A/B)

| Metric | A0 | A1 | A1 vs A0 |
|--------|----|----|----------|
| RPS median | 74647.73 | 78583.61 | **+5.27%** |
| RPS CV | 0.0563 | 0.0439 | improved |
| P50 (ms) | — | — | not extracted in summary script |
| P95 (ms) | 2.86 | 2.75 | **−3.85%** |
| P99 (ms) | 3.71 | 3.59 | **−3.23%** |
| docker-stats CPU % median | 279.47 | 278.44 | ~flat |
| docker-stats RSS MiB median | 28.59 | 29.53 | ~flat |
| TAIL_GATE (+5% max regression) | — | — | **PASS** |

Per-rep RPS (rewrk):

- A0: 74023.68, 81325.92, 76832.82, 69717.59, 74647.73
- A1: 80563.10, 72610.78, 78042.19, 81385.89, 78583.61

Canonical P4 three-way reference (115113Z): ExyonQ RPS median **73096.77** vs NGINX **96446.70** (−24.21%). A0 in this experiment (**74648**) is within harness variance of canonical A0 — **AB validity OK**.

## Bounded value classification

```text
RPS_GAIN_CLASS = SMALL (3% ≤ gain < 7%)
CAP034_RPS_RECOVERABLE_PERCENT ≈ 5.3%
CAP034_CPU_RECOVERABLE_PERCENT ≈ NOT_MATERIAL (docker-stats)
CAP034_CONTEXT_SWITCH_RECOVERABLE_PERCENT = NOT_MEASURED (perf stat unavailable)

DOES_CAP034_EVOLUTION_MATERIALLY_CLOSE_P4_GAP = NO
EXPECTED_MIGRATION_VALUE_EFFECT = PARTIAL

CASE = AB-C34-B
```

A1 improves ExyonQ P4 RPS ~5% with slightly better tails; does **not** close the ~24% competitive gap alone.

## Design / productization

```text
EXPERIMENT_DESIGN_PRODUCTION_READY = PARTIAL
REFACTORING_REQUIRED_BEFORE_PRODUCTIZATION = YES
SINGLE_SOURCE_OF_TRUTH_STATUS = DownstreamFramingDecision + encode_proxy_response_head_framing (experiment)
DUPLICATED_FRAMING_LOGIC_STATUS = reduced vs scattered bools; hop-by-hop skip still parallel to module-api
HOT_PATH_COMPLEXITY_DELTA = moderate (keep-alive loop in wire_dispatch + CL streaming)
MAINTENANCE_RISK = medium — requires full security matrix + dual-arch qualification before product WIP

NEXT_PRODUCT_WIP_PROPOSAL = NONE
  # superseded for primary P4 root work by V044_P4_PROXY_WIRE_BODY_PIPELINE_SLIM (PX-B); Cap034 remains DEFER (owner review: defer vs low-priority productize)
NEXT_MEASUREMENT_WIP_PROPOSAL = NONE

P5_STARTED = NO
P5_TO_P14_FROZEN = YES
HAPROXY_IN_THIS_AB = NOT_MEASURED
```

## Implementation notes (experiment only)

- `crates/exyonq-mod-proxy/src/wire_framing.rs` — `decide_downstream_framing`
- `wire_conn::serve_proxy_single` — known-length streaming + poison on CL mismatch
- `core/src/server/wire_dispatch.rs` — `serve_proxy_wire_keepalive_loop` (per-request Host/route/WAF)
- Harness: `scripts/remote/v044-cap034-framing-keepalive-bounded-value-ab.sh`

**Do not merge** experiment branch into canonical product without owner-authorized productization WIP.
