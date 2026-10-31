# V044 Safe H1 Reuse Terminal Seal

```text
WIP = V044_SAFE_H1_REUSE_TERMINAL_SEAL
PARENT = V044_SAFE_H1_UPSTREAM_REUSE_RESTORATION / URR-A
CASE = REUSESEAL-B
DATE_UTC = 2026-08-30
PLATFORM_PERF = NETCUP_AMD64
PRODUCT_MUTATION = YES_MINIMAL (rustfmt only on URR-A CFD files; no semantic change)
PERFORMANCE_OPTIMIZATION = NO
RIVAL_CAMPAIGN = NO
PHASE_6 = NO
```

## Purpose

Close the claim/evidence gap left by URR-A package-scoped cargo gates, and measure a narrow ExyonQ-only PRE vs POST Profile-A causal performance delta for the safe upstream reuse restoration.

## Workspace reseal (real commands)

| Gate | Command / scope | Result | Classification |
| --- | --- | --- | --- |
| fmt | `cargo fmt --all -- --check` (after mechanical rustfmt on URR CFD files) | PASS | mechanical repair in URR-A files only |
| check | `cargo check --workspace --all-targets` | PASS | — |
| feature matrix | default workspace + `http3-provider-quiche` package runs (canonical) | PASS | scope = supported release matrix |
| feature probe | `cargo check --workspace --all-features` | FAIL | PRE_EXISTING / INVALID matrix (`http3-provider-quinn-legacy` compile_error) |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` (CI/canonical) | PASS | — |
| clippy probe | owner `--all-features` | FAIL | PRE_EXISTING / INVALID (`PROJECT_STATUS`: not a release matrix) |
| test | `cargo test --workspace` | PASS (`TEST_WORKSPACE_EXIT=0`) | full workspace, not CFD-only |

**Claim rule obeyed:** package-level URR-A evidence is not restated as workspace PASS.

## Performance (ExyonQ-only, interleaved PRE/POST)

```text
PROFILE = PHASE5_PROFILE_A
PATH = /api/
CONNECTIONS = 100
WARMUP_S = 20
MEASURE_S = 30
CPUSET = 0-7
SHARDS = 8
ORDER = PRE,POST,POST,PRE,PRE,POST,POST,PRE,PRE,POST,POST,PRE,PRE,POST
PRE_VALID = 7
POST_VALID = 7
EXCLUDED = 0
PRE_BINARY = exyonq-cfd-phase2 fail-closed dataplane (put disabled)
POST_BINARY = URR-A ADR-021 Hybrid F dataplane (safe put)
RUN_ID = seal-20260830T014442Z
```

| Metric (median) | PRE | POST | Delta |
| --- | --- | --- | --- |
| CPU µs/req | 11.987579 | 12.152931 | −1.38% (higher CPU = worse) |
| RPS | 148878.75 | 153921.59 | +3.39% |
| P95 µs | 1250 | 1270 | −1.6% |
| P99 µs | 1500 | 1500 | 0% |
| RSS KB | 5420 | 5432 | −0.22% |

```text
PERFORMANCE_CLASS = PERF-B
REASON = SMALL_OR_NEUTRAL (|CPU| and |RPS| within ~5%; mixed sign)
RIVAL_RECHECK_REQUIRED = NO
P5CPU_A_HISTORICAL_VALIDITY = UNCHANGED
EXT_A_HISTORICAL_VALIDITY = UNCHANGED
POST_REPAIR_COMPETITIVE_FRONTIER = NOT_REMEASURED_AGAINST_RIVALS
```

### Upstream connect / reuse counters

Harness scraped `exyonq_cfd_upstream_connects_total` / `reuse_total` and recorded **0 deltas** for every PRE and POST run. That is an **instrumentation/scrape gap**, not proof of zero connects. Causal reuse remains evidenced by URR-A Phase-4 proofs (`clean_upstream_keepalive_reuses_single_accept`, dirty/trailing isolation) and by post-perf recheck below — **not** by this campaign’s connect medians.

```text
UPSTREAM_CONNECTS_PER_REQ = NOT_MEASURED_IN_SEAL_CAMPAIGN
UPSTREAM_CONNECT_INSTRUMENTATION = GAP
```

## Post-perf security / reuse recheck

```text
POST_PERF_SECURITY_RECHECK = PASS
COMMAND = cargo test -p exyonq-cfd-dataplane --release --test phase4_bodies
RESULT = 11 passed; 0 failed (includes clean_upstream_keepalive_reuses_single_accept,
         delayed_trailing_byte_forces_fresh_accept, checkout_window_trailing_never_parsed_as_response)
POST_PERF_DIRTY_REUSE_COUNT = 0_REQUIRED (covered by phase4 adversarial cases)
```

## Migration value

```text
DOES_SAFE_REUSE_NOW_PROVEN_INCREASE_MIGRATION_VALUE = PARTIAL
```

- Correctness/safety of reuse: proven (URR-A + post-perf phase4).
- Local Profile-A CPU/RPS: small/neutral (PERF-B); not a material efficiency win on local upstream.
- Remote upstream / TLS / ephemeral-port pressure: plausibly higher value, **not remeasured here**.

## Notion reconciliation facts (owner)

```text
NOTION_RECONCILIATION_REQUIRED = YES
FACTS:
- URR-A package cargo gates ≠ full workspace gates; this seal ran real workspace commands.
- Workspace: fmt PASS (after rustfmt), check PASS, canonical feature matrix PASS,
  canonical clippy PASS, test --workspace PASS.
- --all-features check/clippy FAIL = pre-existing invalid matrix (do not treat as URR-caused).
- Perf PRE/POST 7+7 interleaved on Netcup Profile-A → PERF-B (−1.4% CPU, +3.4% RPS).
- Upstream connect counters in seal harness = NOT_MEASURED (gap); reuse correctness still PASS.
- No rivals, no Phase 6, no merge/push/tag/release.
- CASE = REUSESEAL-B
```

## Evidence

```text
LOCAL = .exyonq-local/evidence/safe-h1-reuse-terminal-seal/seal-20260830T004808Z/
NETCUP = /root/exyonq-urr-h1-reuse/.exyonq-local/evidence/safe-h1-reuse-terminal-seal/seal-20260830T014442Z/
```

## Stop

```text
CANONICAL_MERGE = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION = NO
PHASE_6_AUTHORIZED = NO
BROAD_P1_P14_BENCHMARK = NOT_AUTHORIZED
```
