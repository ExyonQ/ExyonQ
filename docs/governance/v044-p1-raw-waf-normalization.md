# V044 P1 RAW WAF normalization

**WIP:** `V044_P1_RAW_WAF_NORMALIZATION` (COMPLETE — CASE A)  
**Product mutation:** NO  
**Harness/config mutation:** YES  
**Harness fix commit:** `7fcf338444a158bf6eeae4e3177f89a23668495c`  
**Terminal HEAD:** `7fcf3384` / tree `7a4d5086`  
**Product authority (unchanged):** `427b1739` (+ `f2e90beb`)  
**Binary SHA-256 (unchanged):** `5a31ccf3decabed3071e587ad1c68fa5fb673f2b130ef8f63e70f7a5c0a13c40`  
**Evidence:** `.exyonq-local/benchmarks/v044-p1-raw-waf-normalization-20260820T205558Z/`

## Contract clarification

```text
absent [waf]  →  product IR default: enabled=true, mode=Monitor
explicit [waf] enabled=false mode="disabled"  →  RAW WAF OFF
```

Product defaults were **correct**. The defect was benchmark methodology assuming absent `[waf]` meant OFF.

## What changed (harness only)

| Path | Change |
| --- | --- |
| `benchmarks/configs/exyonq/bench.toml` | Explicit `[waf] enabled=false mode="disabled"` |
| `scripts/gates/p1-raw-waf-off-gate.sh` | Fail-closed effective-semantics gate |
| `scripts/remote/v044-p1-raw-waf-normalization.sh` | Rebuild + runtime proof + 3-way RAW |

```bash
bash scripts/gates/p1-raw-waf-off-gate.sh --selftest
bash scripts/gates/p1-raw-waf-off-gate.sh --tree .
```

## Runtime proof (before RPS ranking)

```text
WAF_SECTION_PRESENT = YES
WAF_ENABLED_EFFECTIVE = NO
WAF_MODE_EFFECTIVE = DISABLED
EXY_HDR_1001_WARN_EVENTS_DURING_LOAD = 0
SENDFILE_RUNTIME_EXECUTED = YES
SENDFILE_PER_REQ ≈ 1.0006
OPENAT2_PER_REQ ≈ 1.0006
GLOBAL_SESSION_MUTEX_PRESENT = NO
RUNTIME_WAF_OFF_PROOF = PASS
```

## Authoritative three-way P1 RAW (WAF-off)

| Server | Runs (RPS) | Median | CV% |
| --- | --- | --- | --- |
| ExyonQ | 161508, 181709, 135708, 164532, 163179 | **163178.95** | 10.10 |
| NGINX | 147181, 156961, 142221, 129549, 139628 | **142221.03** | 7.08 |
| OLS | 128931, 141452, 138120, 148637, 132186 | **138120.08** | 5.62 |

```text
EXYONQ_VS_NGINX = +14.74% → AHEAD
EXYONQ_VS_OLS   = +18.14% → AHEAD
CLASSIFICATION_BAND = PARITY [-2%, +5%); AHEAD ≥ +5%; MATERIAL_GAP < -2%
P1_OVERALL_COMPETITIVE_CLASSIFICATION = AHEAD
P1_AUTHORITATIVE_RAW_WAF_OFF = YES
P1_STATUS = CLOSED_CURRENT_COMPETITIVE_BASELINE
RESIDUAL_PRODUCT_OPTIMIZATION_REQUIRED = NO
```

## Historical reconciliation (no deletion)

Runs that used absent `[waf]`:

```text
WAF_EFFECTIVE_MODE = MONITOR
RAW_WAF_OFF_VALID = NO
RAW_RANKING_AUTHORITY = INVALID_FOR_TRUE_WAF_OFF_RAW
CAUSAL_EVIDENCE = PRESERVED_WHERE_APPLICABLE
```

Preserved causal history:

- static-TX absolute-form sendfile fix (`f2e90beb`)
- Cap067 SessionTable TLS (+103.4% same-session, `427b1739`)
- WAF discriminator A/B (+31.47% when explicitly disabled)

## Default product vs RAW

| Profile | WAF |
| --- | --- |
| Product default (absent section) | Monitor |
| Canonical P1 RAW (this WIP) | Explicitly disabled |
| DEFAULT_PRODUCT / SECURE_DEFAULT benches | Separate; not changed here |

```text
P2_STARTED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
OWNER_AUTHORIZATION_REQUIRED_BEFORE_P2_OR_ANY_NEW_PRODUCT_OPTIMIZATION = YES
```

## Reporting completeness (latency / CPU / memory / HTML)

**WIP:** `V044_P1_REPORTING_COMPLETENESS` (COMPLETE)  
**Mode:** read-only measurement + reporting (no product mutation)  
**Evidence:** `.exyonq-local/benchmarks/v044-p1-reporting-completeness-20260820T223047Z/`

```text
EXISTING_EVIDENCE_HAS_LATENCY = PARTIAL   # avg/min/max/std YES; true P99 NO (percentiles_estimated)
EXISTING_EVIDENCE_HAS_CPU = NO
EXISTING_EVIDENCE_HAS_MEMORY = NO
EXISTING_EVIDENCE_SUFFICIENT_FOR_REPORT = NO
SOURCE = MIXED
```

Closed competitive RPS baseline is preserved. Latency/CPU/memory come from a same-contract reporting capture using `rewrk --pct` (real histogram table) and container `docker stats`.

| Metric | ExyonQ | NGINX | OLS |
| --- | --- | --- | --- |
| P99 (median of runs, ms) | 1.65 | 1.69 | 1.59 |
| CPU avg % (docker, 8-core cpuset) | 413.08 | 280.54 | 189.66 |
| CPU normalized (avg%/(100×8)) | 0.516 | 0.351 | 0.237 |
| RSS load avg (MiB, docker) | 13.36 | 11.47 | 56.36 |
| RSS load peak (MiB) | 14.09 | 12.68 | 57.72 |

Aggregate latency definition: **median of per-run percentile values**.

Artifacts:

- HTML: `.exyonq-local/benchmarks/v044-p1-reporting-completeness-20260820T223047Z/p1-report.html`
- JSON: `.exyonq-local/benchmarks/v044-p1-reporting-completeness-20260820T223047Z/p1-report.json`
- Also copied next to the closed baseline evidence directory for discoverability

```text
P1_STATUS = CLOSED_CURRENT_COMPETITIVE_BASELINE
RESIDUAL_PRODUCT_OPTIMIZATION_REQUIRED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
```
