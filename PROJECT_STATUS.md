# ExyonQ — Project Status (canonical)

```text
DOCUMENT = PROJECT_STATUS.md
DOCUMENT_ROLE = CANONICAL_PROJECT_STATUS
AUTHORITY = SOLE for current project phase answers
LAST_UPDATED = 2026-08-04
BRANCH = main
PUBLIC_RELEASE = v0.4.3
FINAL_PUBLISHED_RELEASE_HEAD = 3a8af75dd0939e5aae2f4d09b842573499e15a1c
SUPERSEDED_RELEASE_CANDIDATE = f25137bff3b6fc0d4fc1b7f4c6d736b100fae741
PRODUCT_QUALIFIED_REVISION = 2f8e869f97c36e18ca7976d673dd6db2aa16e45e
R2_FINAL_QUALIFICATION_HEAD = 9176c374876d7c4bf503db1c1a02c885bb02fad9
R3_CLOSE_PRODUCT_BASELINE = f0b2d67
GRC_R5_REMEDIATION_STATUS = COMPLETE
PRE_R5_GRC_READINESS = PASS
R4_STATUS = COMPLETE
R4_RESULT = PASS
R4_CLOSED = YES
P2B_OPEN_002 = CLOSED
R5_STATUS = COMPLETE
R5_RESULT = PASS
R5_CLOSED = YES
R6_STATUS = NOT_STARTED
```

Companion authorities:

| Function | Document |
|----------|----------|
| Global roadmap | [`ROADMAP.md`](ROADMAP.md) |
| Documentation map | [`DOCUMENTATION_INDEX.md`](DOCUMENTATION_INDEX.md) |

Only **R\*** identifiers in `ROADMAP.md` answer “what global phase is ExyonQ in?”.

---

## 1. Baseline identity

```text
WORKTREE_AUTHORITY =
  /Volumes/Lexar/Cursor/exyonq-lab
  ROLE = DEVELOPMENT_CANONICAL

GITHUB_RELEASE_AUTHORITY =
  /Volumes/Lexar/Cursor/exyonq-github
  ROLE = GITHUB_STAGING / RELEASE_AUTHORITY
  BRANCH = main
  FINAL_PUBLISHED_RELEASE_HEAD = 3a8af75dd0939e5aae2f4d09b842573499e15a1c
  PUBLIC_RELEASE = v0.4.3

PRODUCT_QUALIFIED_REVISION = 2f8e869f97c36e18ca7976d673dd6db2aa16e45e
CURRENT_BASELINE_STATUS = CLEAN_KNOWN_BASELINE
CURRENT_RELEASE_STATUS = PRIVATE_V043_RELEASE_COMPLETE
BASELINE_DRIFT = NO

PUBLICATION_STATUS = PRIVATE_V043_RELEASE_COMPLETE
GITHUB_RELEASE_STATUS = PUBLISHED
GHCR_V043_STATUS = PUBLISHED
GHCR_V043_DIGEST = sha256:ab02b5ffc3d54407a56b6edbb318d80a077da0f431970984408d82415217f2e5
LATEST_TAG_MUTATED = NO
```

Historical product baseline lineage (unchanged; pre-publication integration line):

```text
HISTORICAL_INTEGRATION_BRANCH = release/v0.4.3-integration
HISTORICAL_PRODUCT_HEAD = 2f8e869f97c36e18ca7976d673dd6db2aa16e45e

CURRENT_PRODUCT_BASELINE =
  v0.4.2 canonical (9742656c8bb1f222694cfa9dd18bf2b76035b94c)
  + Changeset A (5a449c68397acb3e8297875b1506ca016f07759e)
  + Changeset B (fd41b7619f5653af231a401a234c3f4b933456b3)
  + Documentation authority (b3a24ca013a9708a349a4e92e75e124b1d05ccf9)
  + R2 qual docs (664646d1a3c8d5caa643860c236f1e43da72423b)
  + SECINT-001 harness fix (21650408207ef601e3447c2ec0e19afea67da874)
  + wasm fuel helper fix (13c55ad6c0d4746541458b4460b41407d66ff555)
  + R2B wasm qual docs (f5ca35ddd51d3c0fae60146c3bc676577d5d8ece)
  + R2C supported-matrix clippy cleanups (e51cff5f3b4fb48d74db8942131d6262e5e3fcb6)
  + R2D OD-1 handler arity local exceptions (b78ba378b77dd6226708f30785b1d780053d73fb)
  + R2D OD-2 waiver docs/deny/audit (70e78dbbfcf17f3b5bc811a3c4e83ad1be97bbbe)
  + R2D Linux clippy/harness closes through 2f8e869f97c36e18ca7976d673dd6db2aa16e45e
  + R2E KD3 proxy 501 metrics assert gate (2849d06f3dc1a4fbaecd412ab39a569662630eae)
    (test/harness only — product binaries remain @ 2f8e869…)

Cargo.toml / Cargo.lock fingerprints at published release tree are those of
FINAL_PUBLISHED_RELEASE_HEAD (see git tree 42f87a28…).
```

---

## 2. Global phase

R* vocabulary (canonical):

```text
PHASE_LIFECYCLE (*_STATUS) = NOT_STARTED | OPEN | COMPLETE
PHASE_RESULT (*_RESULT)    = PASS | FAIL | ACCEPT | REJECT | PARITY | WIN | LOSS | INCONCLUSIVE | …
```

Lifecycle and outcome are independent. Do not use PASS as a substitute for COMPLETE
in the global R* summary. Do not use COMPLETE as a substitute for PASS in
qualification/competitive results.

```text
PROJECT_GLOBAL_PHASE =
  R5 COMPLETE (PASS) — private v0.4.3 published
  BASIC_PRODUCT_COMPLETENESS_GATE = PASS (accepted; unchanged)
  GRC_P0 = COMPLETE (audit snapshot; does not override product/R3 truths)
  GRC_R5_BLOCKER_REMEDIATION = COMPLETE (PRE_R5_GRC_READINESS=PASS)
  R4 = COMPLETE (PASS — K8S P2A/P2B replay + OPEN-002 CLOSED)
  R6 = NOT_STARTED

ROADMAP_POSITION = R5 COMPLETE — see ROADMAP.md; R6 NOT_STARTED

R0 = COMPLETE
R1 = COMPLETE
R2 = COMPLETE (PASS)
R3 = COMPLETE (PASS — PARITY)
R4 = COMPLETE (PASS)
R5 = COMPLETE (PASS)
R6 = NOT_STARTED

R0_STATUS = COMPLETE
R1_STATUS = COMPLETE
R2_STATUS = COMPLETE
R2_RESULT = PASS
R2_CLOSED = YES
R3_STATUS = COMPLETE
R3_RESULT = PASS
R3_CLOSED = YES
R3_COMPETITIVE_RESULT = PARITY
R4_STATUS = COMPLETE
R4_RESULT = PASS
R4_CLOSED = YES
R4_OPENED = YES
R5_STATUS = COMPLETE
R5_RESULT = PASS
R5_CLOSED = YES
R5_OPENED = YES
R6_STATUS = NOT_STARTED

PROJECT_PHASE_1_STATUS = NOT_CLOSED
PROJECT_PHASE_2_STATUS = NOT_STARTED
```

---

## 2b. R5 OPEN (post-bump freeze requalification — 2026-08-04)

Historical freeze/requalification record (pre-publication). Current lifecycle is in §2c.

```text
R5_STATUS = OPEN   # HISTORICAL snapshot at freeze requalification time
R5_OPENED = YES
TARGET_RELEASE_VERSION = 0.4.3
R5_VERSION_BUMP_STATUS = PASS
VERSION_IDENTITY_STATUS = COHERENT_FOR_V043_RELEASE
VERSION_BUMP_COMMIT = e8fa4dca9336bb8293b56ca8ad47e24bfd4fd792
POST_BUMP_PRODUCT_TIP = 7ca6d40c45cce16c0eb480c64e2a2b29b26ae3a6
PRE_BUMP_FREEZE_HEAD = e63325d53f1fb0848a37e55f8a52fc9f6409739e
PRE_BUMP_RELEASE_CANDIDATE_HEAD = 2d2ea8109efd47e2b87dde9faf152741d8484b91
FREEZE_QUALIFICATION_INVALIDATED_BY_BUMP = YES (pre-bump evidence historical)
R5_POST_BUMP_QUALIFICATION = PASS
R5_FREEZE_QUALIFICATION = PASS
R5_FREEZE_READY = YES
R5_DARWIN_STATUS = PASS
R5_AMD64_STATUS = PASS
R5_ARM64_STATUS = PASS
WRR_PROTECTORS_AMD64 = PASS
WRR_PROTECTORS_ARM64 = PASS
ARCHITECTURE_COMPILER_STATUS = ACCEPT
WRR_PERFORMANCE_COMPILER_STATUS = ACCEPT
SECURITY_COMPILER_STATUS = CONDITIONAL ACCEPT (RUSTSEC-2026-0222 unfixed; Wasmtime 45.0.2; waiver → v0.4.4)
R5_SECURITY_AUDIT_STATUS = PASS
BASIC_PRODUCT_COMPLETENESS_GATE = PASS
NO_SMOKE_AS_PROOF_GATE_RESULT = GREEN
SMOKE_TEST_POLICY = DIAGNOSTIC_ONLY_NON_AUTHORITATIVE (may not close gates)
PRE_R5_GRC_READINESS = PASS
FINAL_ARTIFACT_VERSION_COHERENCE = PASS
OCI_INDEX_CREATED = YES
R5_RELEASE_EXECUTION_READY = YES
R5_RELEASE_EXECUTION_AUTHORIZED = NO   # HISTORICAL — superseded by §2c publication
PRODUCTION_SIGNING_EXECUTED = NO      # HISTORICAL — superseded by §2c publication
PUSH = NO                             # HISTORICAL
TAG = NO                              # HISTORICAL
RELEASE = NO                          # HISTORICAL
PUBLICATION_STATUS = FORBIDDEN        # HISTORICAL snapshot
```

Pre-bump freeze tip `e63325d…` remains valid **historical** evidence only.
Post-bump evidence: `.exyonq-local/release/r5-postbump-e8fa4dca9336/` (local only).

---

## 2c. R5 CLOSED — private v0.4.3 publication (2026-08-04)

```text
R5_STATUS = COMPLETE
R5_RESULT = PASS
R5_CLOSED = YES
R6_STATUS = NOT_STARTED

PUBLIC_RELEASE = v0.4.3
VERSION = 0.4.3
RELEASE_TAG = v0.4.3
FINAL_PUBLISHED_RELEASE_HEAD = 3a8af75dd0939e5aae2f4d09b842573499e15a1c
FINAL_RELEASE_TREE = 42f87a28384155c8d2f81a4bb6770f9e39c56fbb
SUPERSEDED_RELEASE_CANDIDATE = f25137bff3b6fc0d4fc1b7f4c6d736b100fae741
SUPERSEDED_CLASSIFICATION = SUPERSEDED_RELEASE_CANDIDATE

GITHUB_RELEASE_STATUS = PUBLISHED
GITHUB_RELEASE_URL = https://github.com/ExyonQ/ExyonQ/releases/tag/v0.4.3
GHCR_V043_STATUS = PUBLISHED
GHCR_V043_DIGEST = sha256:ab02b5ffc3d54407a56b6edbb318d80a077da0f431970984408d82415217f2e5
LATEST_TAG_MUTATED = NO

SSH_TAG_SIGNATURE_STATUS = PASS
COSIGN_BLOB_SIGNATURE_STATUS = PASS
COSIGN_OCI_SIGNATURE_STATUS = PASS
R5_RELEASE_SIGNATURE_SET = PASS

RUSTSEC_2026_0222_EXCEPTION_STATUS = ACTIVE_WAIVER
VULNERABILITY_FIXED = NO
WAIVED_FOR_V043 = YES
EXPIRY_REVIEW = v0.4.4

PUBLICATION_STATUS = PRIVATE_V043_RELEASE_COMPLETE
```

---

## 3. R2 qualification summary (2026-07-31 → R2E 2026-08-02)

```text
R2_STATUS = COMPLETE
R2_RESULT = PASS
R2_CLOSED = YES
R2_QUALIFICATION_STATUS = PASS
R2D_STATUS = SUPERSEDED_BY_R2E
R2E_STATUS = PASS
R2E_RESULT = PASS
ENVIRONMENT = Darwin + Netcup amd64 + Oracle arm64 + OCI dual-arch local
LINUX_AMD64_QUALIFICATION_STATUS = PASS (product @ 2f8e869…)
LINUX_ARM64_QUALIFICATION_STATUS = PASS (product @ 2f8e869…)
OCI_AMD64_BUILD_SMOKE = HISTORICAL_R2D_CELL class=A version-only (NOT_GATE_CLOSING under NO-SMOKE)
OCI_ARM64_BUILD_SMOKE = HISTORICAL_R2D_CELL class=A version-only (NOT_GATE_CLOSING under NO-SMOKE)
OCI_RUNTIME_E2E_STATUS = PASS_REAL_E2E (dual-arch NS5; HISTORICAL R2 evidence under .exyonq-local/tmp/; real OCI E2E, NOT_GATE_CLOSING for R5)
OCI_INDEX_PUBLICATION_STATUS = NOT_EXECUTED_NOT_AUTHORIZED
DARWIN_WORKSPACE_STATUS = PASS (R2E workspace run 3; evidence .exyonq-local/tmp/r2e-20260801/)
AMD64_TARGETED_KD3_HARNESS = PASS (Netcup; binary 10/10 + race501 2/2)
ARM64_TARGETED_KD3_HARNESS = PASS (Oracle; binary 10/10 + race501 2/2)
WC7D_PROGRESS_CHECK_EXIT_127 = SHELL_TYPO_ONLY (rg …|running; not product/test)
WC7D_PRODUCT_FAILURE = NO
WC7D_TEST_FAILURE = NO
WORKSPACE_RUN_3 = PASS
LATE_NOTIFICATIONS = STALE_IGNORE
R2_REOPEN_REQUIRED = NO
R2E_REOPEN_REQUIRED = NO

SMOKE_TEST_POLICY = DIAGNOSTIC_ONLY_NON_AUTHORITATIVE
SMOKE_TEST_MAY_CLOSE_CAPABILITY = NO
SMOKE_TEST_MAY_QUALIFY_RELEASE = NO
SMOKE_TEST_MAY_ADMIT_BENCHMARK = NO
REAL_E2E_REQUIRED_FOR_CAPABILITY_PASS = YES
NO_SMOKE_AS_PROOF_GATE = ACTIVE (scripts/gates/no-smoke-as-proof-gate.sh)
NO_SMOKE_AUDIT_STATUS = NS0_NS8_DONE; NS5_CORE+OCI_PASS; HTTP3_POST_BODY_FIXED_DUAL_ARCH
NO_SMOKE_R2_IMPACT = PASS_REQUIRES_ADDITIONAL_REAL_E2E
BASIC_PRODUCT_COMPLETENESS_STATUS = PASS
BASIC_PRODUCT_COMPLETENESS_GATE = PASS
HTTP3_POST_BODY_FIX_COMMIT = 8b702a2
HTTP3_POST_BODY_AMD64 = PASS
HTTP3_POST_BODY_ARM64 = PASS
HTTP3_POST_BODY_EVIDENCE = .exyonq-local/tmp/bpc-20260802/

R3_STATUS = COMPLETE
R3_RESULT = PASS
R3_CLOSED = YES
R3_COMPETITIVE_RESULT = PARITY
R3_READY_TO_CLOSE = YES
R3_RESUME_ALLOWED = YES
R3_RESUME_AUTHORIZED = YES
R3_PRODUCT_BASELINE = f0b2d67
R3_CANONICAL_HEAD = f0b2d67
LATE_R3_NOTIFICATIONS = STALE_IGNORE
ABORTED_R3B_RERUN = CLOSED_ABORTED
OLD_R3B_PARTIAL_OUTPUT = NON_CANONICAL
SUPERSEDED_R3_CAPACITY_OUTPUT = NON_CANONICAL
FIRST_DUAL_ARCH_RUN = INVALID_HARNESS_EMFILE
FIRST_DUAL_ARCH_RUN_CANONICAL = NO
NOFILE_FIX_APPLIED = YES
CANONICAL_RERUN_COMPLETED = YES
EVIDENCE_PULL_RSYNC_ABORTED = YES
EVIDENCE_PULL_ABORT_AFFECTS_RESULT = NO
REMOTE_DIAGNOSIS_COMPLETED = YES
R3A_STATUS = PASS (infra reused; class A)
R3B_STATUS = PASS
R3B_AMD64_MAX_VALID_RPS = 75000
R3B_ARM64_MAX_VALID_RPS = 40000
R3B_EVIDENCE = .exyonq-local/tmp/r3-20260803/
R3C_STATUS = PASS
R3C_AMD64_SAFE_RPS = 62500
R3C_ARM64_SAFE_RPS = 31250
R3C_EVIDENCE = .exyonq-local/tmp/r3-20260803/
R3D_STATUS = PASS
R3D_TOPOLOGY_NEUTRALITY = PASS
R3E_STATUS = PASS_VALID
R3E_EXECUTED = YES
R3E_EXYONQ_REVISION = f0b2d67
R3E_AMD64_EXYONQ = 39481.6 median RPS @ requested 40000
R3E_AMD64_OLS = 39482.3 median RPS @ requested 40000
R3E_AMD64_CLASSIFICATION = PARITY
R3E_ARM64_EXYONQ = 19747.7 median RPS @ requested 20000
R3E_ARM64_OLS = 19747.7 median RPS @ requested 20000
R3E_ARM64_CLASSIFICATION = PARITY
R3E_OVERALL_CLASSIFICATION = PARITY
R3E_NOTE = fixed-rate PARITY (not open-loop ceiling); first EMFILE run NON_CANONICAL / HISTORICAL_ONLY
R3E_EVIDENCE = .exyonq-local/tmp/r3-20260803/{amd64,arm64}-r3e/ (canonical rerun)
R3F_STATUS = PASS
R3F_RECONCILIATION = study/reports/R3F_RECONCILIATION.md (+ local copy under evidence dir)
PERFORMANCE_COMPILER_STATUS = ACCEPT
R3_HTML_UPDATED = YES (study/reports/r3-exyonq-vs-ols-fixed-rate-parity.html; also docs/benchmarks/ local gitignored copy)
R3_PUBLIC_CLAIM_ALLOWED = YES (fixed-rate PARITY only; PUBLICATION_STATUS still FORBIDDEN for release/tag)
R4_READY_TO_OPEN = YES
R4_OPENED = NO
R5_OPENED = NO
PRODUCT_CODE_CHANGED = NO
NO_BASIC_PRODUCT_DEFECTS = YES
DEFERRED_BASIC_FUNCTIONAL_ITEMS = 0
DEFERRED_UNJUSTIFIED_ITEMS = 0
NO_AUTHORITATIVE_SMOKE_GATES = YES
R3_NO_SMOKE_CONTRADICTION = STALE_FIXED_OR_SUPERSEDED

NO_SMOKE_NS5_STATUS = CORE+OCI+HTTP3_POST PASS_REAL_E2E (dual-arch)
NO_SMOKE_NS5_EVIDENCE = .exyonq-local/tmp/no-smoke-audit-20260802/ + .exyonq-local/tmp/bpc-20260802/
FULL_RELEASE_QUALIFICATION = PASS_FOR_R2 (R5 freeze still separate)
EVIDENCE = .exyonq-local/tmp/r2d-20260801/ + .exyonq-local/tmp/r2e-20260801/
```

### Demonstrated local results

| Gate | Status | Notes |
|------|--------|-------|
| `cargo fmt --check` | PASS | |
| `cargo check --workspace --all-targets` | PASS | warnings only (unused imports) |
| `cargo test --workspace --all-targets` | PASS (Darwin R2E) | KD3 501-counter race fixed via `proxy_metrics_assert_guard`; workspace-3 EXIT=0 |
| `cargo test -p exyonq-mod-proxy` | PASS | Linux dual-arch; Darwin focused reconfirm in evidence |
| `cargo test -p exyonq-core` | PASS | Linux dual-arch; kd3 serial/parallel binary PASS on Darwin when not under workspace contention |
| `plan12_fastcgi_cache_test` | PASS | dual-arch |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS | OD-1 + Linux-only lint closes; Darwin `DARWIN_CLIPPY_WS_EXIT=0` |
| `cargo clippy -p exyonq-wasm-host --all-targets -- -D warnings` | PASS | Darwin + Linux |
| `cargo clippy --all-features` | INVALID | not a supported release matrix |
| `cargo audit` / `cargo deny` | WAIVED (R2D OD-2) | `RUSTSEC-2026-0222`; **not** fixed; Wasmtime 45.0.2 |
| Private-material | N/A scanner path missing | selftest PASS via `scripts/test-tls/exyonq-sec-private-material-zero.sh` |
| Release binaries `--locked` | PASS | Netcup + Oracle |
| OCI build+version (HISTORICAL R2D) | NOT_GATE_CLOSING | class A version-only @ `2f8e869…`; runtime E2E required |

### SECINT-001 (R2A — resolved as harness race)

```text
R2A_SECINT001_STATUS = FIXED (TEST_SYNCHRONIZATION; not a product H3/proxy bug)
FAILURE_CLASS = TEST_SYNCHRONIZATION
PACKAGE = exyonq-integration-tests
TEST = security_secint001_h3_proxy_get_with_upstream
ROOT_CAUSE =
  concurrent ServerState::new (static.toml → empty proxy slots) called
  bind_proxy_compiled_slots on the process-wide ProxyRuntime while SECINT-001
  held a live upstream binding → ProxyRuntime::target_for miss / wrong target
  → ProxyDispatchOutcome::BadGateway → HTTP 502
FIX_COMMIT = 21650408207ef601e3447c2ec0e19afea67da874
FIX_SCOPE = tests/integration/security.rs only
  (SPAWN_SERVER_GATE extended to security_http3_rejects_post +
   security_reload_invalid_config_keeps_snapshot)
PRODUCT_CODE_CHANGED = NO
VALIDATION =
  SECINT-001 10/10 isolated PASS
  security suite stress 15/15 PASS (previously flaked under parallel load)
  H1 proxies_to_configured_upstream PASS
  p4_no_implicit_api_cache + project_integrity_env_delta + plan12_proxy_cache PASS
  cargo test -p exyonq-mod-proxy / exyonq-core / exyonq-integration-tests PASS
  cargo test --workspace --all-targets (pre-R2B): SECINT-001 ok; FAIL_OTHER on
    exyonq-wasm-host bench invoke_latency (FuelExhausted) — see R2B
HOST = Darwin local (NOT_LINUX_EVIDENCE)
EVIDENCE = .exyonq-local/tmp/r2a-secint001-20260801/
```

### invoke_latency FuelExhausted (R2B — resolved)

```text
R2B_WASM_STATUS = FIXED
ROOT_CAUSE_CLASS = PRODUCT_FUEL_POLICY_BUG
ROOT_CAUSE_FILE = wasm/exyonq-wasm-host/src/lib.rs
ROOT_CAUSE_SYMBOL = invoke_i32_with_fuel (+ is_fuel_error)
ROOT_CAUSE_DESCRIPTION =
  create_engine() enables consume_fuel + epoch_interruption; fuel-only helper
  set fuel but never store.set_epoch_deadline → Wasmtime default epoch deadline 0
  → immediate interrupt trap → misclassified as FuelExhausted via
  is_fuel_interrupt_trap ("interrupt"). is_fuel_error also used to_string()
  (outer backtrace only); fixed to format!("{err:#}") for real fuel traps.
REGRESSION_ORIGIN = PRE_EXISTING (present on v0.4.2 base; not Changeset A/B)
PRODUCT_FAILURE_PROVEN = YES (helper + error mapping; production
  call_on_request_headers already set_epoch_deadline(5))
BENCHMARK_FAILURE_PROVEN = NO (bench expectations correct; harness exposed helper bug)
CARGO_MATRIX_ISSUE_PROVEN = NO (Criterion harness=false under --all-targets is expected)
FIX_COMMIT = 13c55ad6c0d4746541458b4460b41407d66ff555
FIX_SCOPE = wasm/exyonq-wasm-host/src/lib.rs only
  (set_epoch_deadline in invoke_i32_with_fuel; is_fuel_error cause-chain;
   regression test noop_invoke_succeeds_with_fuel_budget)
FUEL_SAFETY_WEAKENED = NO
CARGO_TOML_CHANGED = NO
VALIDATION =
  Criterion invoke_latency --test ×10 PASS
  cargo test -p exyonq-wasm-host PASS (lib + plugin_load)
  cargo test --workspace --all-targets PASS (WORKSPACE=0)
  cargo fmt --check PASS; cargo check --workspace --all-targets PASS (pre-docs)
HOST = Darwin local (NOT_LINUX_EVIDENCE)
EVIDENCE = .exyonq-local/tmp/r2b-wasm-20260801/
WORKSPACE_FUNCTIONAL_STATUS = PASS (local Darwin)
R2_STATUS_AFTER_WASM = FAIL (clippy, RUSTSEC-2026-0222, Linux amd64/arm64, container)
```

### Clippy + RUSTSEC (R2C — 2026-08-01; R2D owner decisions accepted)

```text
R2C_STATUS = COMPLETE_WITH_RESIDUALS (superseded by R2D OD-1/OD-2 below)
EVIDENCE = .exyonq-local/tmp/r2c-clippy-20260801/
R2D_EVIDENCE = .exyonq-local/tmp/r2d-20260801/

V043_CLIPPY_GATE (required) =
  cargo clippy --workspace --all-targets -- -D warnings
  cargo clippy -p exyonq-wasm-host --all-targets -- -D warnings

V043_CLIPPY_GATE (optional supported, separate runs) =
  s2n+quiche dual: cargo clippy -p exyonq-mod-http3 --all-targets
    --features http3-provider-quiche -- -D warnings
  quiche-only: cargo clippy -p exyonq-core --no-default-features
    --features http3-provider-quiche --all-targets -- -D warnings
    (and matching exyonq / exyonq-mod-http3 packages as needed)

ALL_FEATURES_MONOLITHIC =
  NOT_A_VALID_RELEASE_MATRIX
  Proven: cargo check --workspace --all-features → compile_error!
  (http3-provider-quinn-legacy cannot combine with s2n or quiche)

CLIPPY_ARITY_DECISION = DEFER_STRUCTURAL_REFACTOR_WITH_NARROW_LOCAL_EXCEPTION
CLIPPY_GLOBAL_RELAXATION = NO
HANDLER_ARITY_REFACTOR_V043 = NO
TECH_DEBT_HANDLER_ARITY = DEFERRED_POST_V043
CLIPPY_ARITY_EXCEPTIONS =
  core/src/server/handler.rs::dispatch_core
  core/src/server/handler.rs::proxy_contract_target
OD1_COMMIT = b78ba378b77dd6226708f30785b1d780053d73fb
DEFAULT_CLIPPY_STATUS = PASS (after OD-1; reconfirm in R2D Phase A)

RUSTSEC_DECISION = OPTION_B
RUSTSEC_2026_0222 =
  Package = wasmtime 45.0.2
  Severity = Low / CVSS 3.8
  VULNERABILITY_FIXED = NO
  ADVISORY_WAIVED_FOR_V043 = YES
  RUSTSEC_WAIVER_EXPIRY = v0.4.4 dependency review
  WASMTIME_UPDATE_V043 = NO
  WASMTIME_UPDATE_POST_V043 = REVIEW_REQUIRED (≥46.0.2)
  CURRENT_PRODUCT_REACHABILITY = NOT_REACHABLE_UNDER_VALIDATED_MODEL
  RUSTSEC_WAIVER_ASSUMPTION_VERIFIED = YES
    (WasmPlugin owns one Engine; Store/Module from that Engine;
     no public API accepting arbitrary cross-Engine Wasmtime objects)
  Waiver materialization =
    deny.toml [advisories].ignore RUSTSEC-2026-0222
    .cargo/audit.toml advisories.ignore RUSTSEC-2026-0222
  Cargo.toml / Cargo.lock NOT changed

R2_STATUS_AFTER_R2E =
  OD-1/OD-2 closed; Linux amd64+arm64+OCI local PASS @ 2f8e869…;
  Darwin KD3 501-counter race closed (`proxy_metrics_assert_guard`);
  Darwin workspace `--all-targets` PASS (R2E ws-3) → R2_STATUS=COMPLETE / R2_RESULT=PASS
```

### Legacy smoke scripts (NON_AUTHORITATIVE_DIAGNOSTIC_ONLY)

| SCRIPT | CAPABILITY | STATUS | ENVIRONMENT | FAILURE_CLASS |
|--------|------------|--------|-------------|---------------|
| `kd2-5-static-smoke.sh` | static | SKIP | Darwin | NOT_APPLICABLE (Linux-only) |
| `kd3-proxy-smoke.sh` | proxy | SKIP | Darwin | NOT_APPLICABLE (Linux-only) |
| `plan08-fcgi-real-smoke.sh` | FastCGI | SKIP | Darwin | NOT_APPLICABLE (Linux-only) |
| `p13a-tls-alpn.sh` | TLS ALPN | PASS | Darwin | — |
| `p13a-http2-multiplex.sh` | HTTP/2 | FAIL | Darwin | TEST_HARNESS_FAILURE / fixture (`payloads/www` has no `index.html`) |
| `p13a-tls-reload.sh` | reload | FAIL | Darwin | TEST_HARNESS_FAILURE (CERT_PEM empty before `cp`) |
| `p13b-http3-runtime-foundation.sh` | HTTP/3 | SKIP | Darwin | EXTERNAL_DEPENDENCY_MISSING (curl without HTTP/3) |

### Integrity residuals (product path)

```text
BENCH_API_CACHE_PATHS = ABSENT from product code (test comment only)
EXYONQ_BENCH_CACHE_HEADERS = retired / env-delta covered; no synthetic mutation enabled
version claim = 0.4.3 (workspace; publication still FORBIDDEN)
```

---

## 4. Functional status

Status vocabulary: `PASS_REAL_E2E` | `PASS` | `CONDITIONAL` | `PARTIAL` | `BLOCKED` | `NOT_STARTED` | `DEFERRED` | `UNKNOWN`

| AREA | STATUS | EVIDENCE | OPEN BLOCKERS | NEXT ACTION |
|------|--------|----------|---------------|-------------|
| core HTTP | PASS | Netcup+Oracle core/workspace; Darwin R2E workspace-3 PASS | — | — |
| static serving | PASS_REAL_E2E | `scripts/e2e/static-e2e.sh` dual-arch NS5 | — | protect |
| reverse proxy | PASS_REAL_E2E | `scripts/e2e/proxy-e2e.sh` → kd3 dual-arch NS5 | deepen optional | protect |
| explicit cache policy | PASS | plan12 + p4 integrity dual-arch | — | — |
| FastCGI / PHP-FPM | PASS_REAL_E2E | plan08 + POST + missing-script→502 dual-arch | — | protect |
| TLS | PASS_REAL_E2E | `scripts/e2e/tls-e2e.sh` ALPN+HTTPS GET+reload dual-arch | — | protect |
| HTTP/2 | PASS_REAL_E2E | `scripts/e2e/http2-e2e.sh` multiplex+hash dual-arch | — | protect |
| HTTP/3 / QUIC | PASS_REAL_E2E | `http3-post-body-e2e` dual-arch after fix `8b702a2` | — | protect |
| reload / drain | PASS_REAL_E2E | tls-e2e reload suite dual-arch | — | protect |
| CLI / config | PASS | dual-arch | — | — |
| module API | PASS | Linux tests | — | — |
| wasm host | PASS | tests + bench test-mode dual-arch | Wasmtime RUSTSEC waived | v0.4.4 dep review |
| htaccess/compat | PARTIAL | documented subset; plan05b/plan11/nginx import tests | not full Apache | keep subset claim |
| security/release | PARTIAL | RUSTSEC waived OD-2; no v0.4.3 audit doc | freeze audit | R5 later (not basic core) |
| container/packaging | PASS_REAL_E2E | `oci-runtime-e2e` dual-arch NS5 | publication forbidden | protect |
| benchmark P1–P3 | PASS | closed tracks | — | protect |
| benchmark P4 | PASS | R3E fixed-rate PARITY dual-arch @ f0b2d67; perf-compiler ACCEPT | EMFILE first run NON_CANONICAL | protect; no rerun |
| Kubernetes P2A/P2B | PASS_REPLAYED (R4) | IR+WRR on canonical; OPEN-002 CLOSED | P2C not opened | out of BPC core |
| v0.4.3 | PARTIAL | project/release phase | R4/R5 + GRC release gaps | not a product capability |

### Benchmark / K8S (unchanged truths)

```text
BV04_P1_STATUS = CLOSED
BV04_P2_STATUS = CLOSED
BV04_P3_STATUS = CLOSED
BV04_P1_P3_INVALIDATED_BY_P4_CACHE_BUG = NO
BV04_P4_COMPETITIVE_STATUS = SUPERSEDED_BY_R3E_PARITY (fixed-rate; not BV04 Mode A ceiling)
BV04_P4_READY_FOR_LOAD = N/A (R3 closed via fresh harness on f0b2d67)

K8S_P2B_STATUS = CLOSED_REPLAYED_ON_CANONICAL
P2B_OPEN_002 = CLOSED
K8S_PRODUCTIVE_INTEGRATION_STATUS = ACCEPTED_R4
K8S_P2C_READY_TO_OPEN = YES
K8S_P2C_OPENED = NO
```

---

## 5. Capability inventory (implementation evidence)

| Capability | Status | Evidence basis |
|------------|--------|----------------|
| HTTP/1.1 | IMPLEMENTED | core + integration tests |
| static serving | IMPLEMENTED | `exyonq-mod-static`, dispatch tests |
| routing | IMPLEMENTED | BackendTable / RouteDecision |
| reverse proxy | IMPLEMENTED | `exyonq-mod-proxy` + tests |
| explicit cache policy | IMPLEMENTED | `cache_policy` + plan12 |
| FastCGI / PHP-FPM | IMPLEMENTED | `exyonq-mod-fastcgi` + plan12; Linux real FPM HISTORICAL cell (POST E2E pending) |
| TLS | IMPLEMENTED | `exyonq-mod-tls` + p13a ALPN |
| HTTP/2 | IMPLEMENTED | ALPN/h2 path; local multiplex smoke blocked by fixture |
| HTTP/3 / QUIC | IMPLEMENTED | HISTORICAL H3 foundation/SECINT (NOT_GATE_CLOSING alone); product-e2e script exists |
| reload / drain | IMPLEMENTED | Linux tls-reload PASS after `ensure_ephemeral` harness |
| CLI | IMPLEMENTED | `exyonq` / `exyonqctl` |
| configuration parsing | IMPLEMENTED | config IR/surface/merge |
| module API | IMPLEMENTED | module-api + modules |
| htaccess/import | IMPLEMENTED | htaccess + nginx compat tests |
| security controls | PARTIAL | gates exist; advisory + H3 SECINT open |
| container/runtime packaging | PARTIAL | Dockerfile + nfpm; OCI not executed here |

---

## 6. Proposed v0.4.3 scope (HISTORICAL proposal; later published)

```text
SECTION_CLASSIFICATION = HISTORICAL_SCOPE_PROPOSAL
PUBLISHED_AS = private v0.4.3 @ 3a8af75… (2026-08-04)
```

```text
INCLUDE =
  Changeset A
  Changeset B
  canonical PROJECT_STATUS / ROADMAP / DOCUMENTATION_INDEX
  qualification-only documentation updates
  R3 fixed-rate P4 PARITY closeout (docs/evidence; product @ f0b2d67 baseline)

EXCLUDE =
  Kubernetes P2C / controller / Helm / CRD (until owner admits P2C)
  BV04 harness changes
  legacy lab imports
  dependency upgrades (unless separately authorized)
  P14C experimental changes
  PI-007 unless separately authorized
  GRC-P1+ registers/catalogs (not authorized)
```

Release doc updates **required later** (not edited in R2):

```text
CHANGELOG_UPDATE_REQUIRED = YES (Unreleased / 0.4.3 notes for A+B)  # DONE at publication
README_UPDATE_REQUIRED = YES (link PROJECT_STATUS; version narrative)
SECURITY_MD_UPDATE_REQUIRED = REVIEW  # DONE at post-release docs reconciliation
RELEASE_CHECKLIST_UPDATE_REQUIRED = YES (when freeze authorized)
```

### Dependency finding

```text
RUSTSEC-2026-0222 wasmtime 45.0.2 — Severity Low (3.8)
RUSTSEC_DECISION = OPTION_B (owner accepted R2D)
VULNERABILITY_FIXED = NO
ADVISORY_WAIVED_FOR_V043 = YES
CURRENT_PRODUCT_REACHABILITY = NOT_REACHABLE_UNDER_VALIDATED_MODEL
WASMTIME_UPDATE_V043 = NO
FOLLOW_UP = evaluate Wasmtime ≥46.0.2 after v0.4.3 with API/behavior regression testing
EXPIRY_REVIEW = v0.4.4
```

---

## 7. Known truths

1. P1–P3 remain valid.  
2. P4 competitive benchmark on baseline `f0b2d67` is **closed** as dual-arch fixed-rate **PARITY** (R3E); EMFILE first attempt is NON_CANONICAL / HISTORICAL_ONLY.  
3. Implicit proxy cache removed; bench header mutation retired.  
4. Focused proxy/core/plan12 tests PASS on the product-qualified revision.  
5. R2D historically: Darwin workspace `--all-targets` FAIL (KD3 harness) blocked
   R2 close. R2E closed that cell → R2_STATUS=COMPLETE / R2_RESULT=PASS;
   R2E_RESULT=PASS at `9176c37`.
   Late shell notifications (including wc7d progress-check exit 127 typo) are
   STALE_IGNORE — not a reopen reason.
6. Kubernetes P2B not integrated.  
7. Private **v0.4.3** is **published** (`3a8af75…` / tag `v0.4.3`); R5 COMPLETE; R6 NOT_STARTED.  
8. `--all-features` Clippy/check is not a valid v0.4.3 release matrix.  
9. RUSTSEC-2026-0222 is **waived**, not fixed; Wasmtime remains 45.0.2 (expiry/review v0.4.4).
10. Smoke is DIAGNOSTIC_ONLY_NON_AUTHORITATIVE; cannot close capability/release/benchmark.
11. NO_SMOKE_R2_IMPACT = PASS_REQUIRES_ADDITIONAL_REAL_E2E (R2 historical PASS retained; NS5 real E2E later closed for core/OCI/HTTP3 POST).
12. R3 paused-for-NO-SMOKE → BPC PASS → R3 resume → R3E PARITY → performance-compiler ACCEPT → R3 CLOSED. Prior “resume vs NO_SMOKE” contradiction is **STALE_FIXED_OR_SUPERSEDED**.
13. GRC-P0 is an accepted **audit snapshot**; its findings do **not** automatically override newer canonical product/R3 state. GRC-P1 is **not** authorized.
14. Basic Product Completeness PASS and GRC maturity gaps are **separate dimensions**.

## 8. Explicit non-claims

```text
No claim that RUSTSEC-2026-0222 is fixed (waived only; ACTIVE_WAIVER for v0.4.3; EXPIRY_REVIEW=v0.4.4).
No claim that fixed-rate PARITY is an open-loop ceiling WIN/LOSS.
No claim that Kubernetes is integrated.
No claim that Phase 2 / R6 has started.
No claim that smoke PASS closes a capability, release, or benchmark admission.
No claim that OCI version-only cell proves container runtime qualification.
No claim that GRC-P0 Critical gaps are product correctness defects.
No claim that GRC registers / EXQ-* IDs are canonical (PROVISIONAL until GRC-P2 admission).
No claim that `latest` was updated by v0.4.3 (LATEST_TAG_MUTATED=NO).
```

---

## 9. Blockers and next actions

```text
PRODUCT_BLOCKERS = NONE_OBSERVED
NO_BASIC_PRODUCT_DEFECTS = YES
BASIC_PRODUCT_COMPLETENESS_GATE = PASS

WORKSPACE_BLOCKERS = NONE
  R2E closed KD3 process-wide PROXY_HTTP_501 race via proxy_metrics_assert_guard
  (mirrors fcgi_metrics_assert_guard). Darwin workspace-3 EXIT=0.

SECURITY_BLOCKERS =
  RUSTSEC-2026-0222 waived for v0.4.3 only (not fixed; Wasmtime 45.0.2)
  no Critical/High open under waived advisory model
  GRC release-governance gaps remain CURRENT (see §10) — separate from product PASS

PACKAGING_BLOCKERS =
  OCI multiarch index / GHCR push NOT_AUTHORIZED (local images validated @ 2f8e869…)

INFRASTRUCTURE_BLOCKERS = NONE for R2/R3 cells

OWNER_DECISIONS_REQUIRED =
  R4 open authorization (optional next); /docs/ SoT decision before GRC/docs remediation;
  GRC-P1 separately if/when authorized; stop before product-code changes / R5 / publish
```

```text
R3_STATUS = COMPLETE
R3_RESULT = PASS
R3_CLOSED = YES
R3_COMPETITIVE_RESULT = PARITY
R3_OPENED = YES
R3_READY_TO_CLOSE = YES
R3_RESUME_ALLOWED = YES
R3_RESUME_AUTHORIZED = YES
R3_PRODUCT_BASELINE = f0b2d67
R3_CANONICAL_HEAD = f0b2d67
BASIC_PRODUCT_COMPLETENESS_GATE = PASS

LATE_R3_NOTIFICATIONS = STALE_IGNORE
ABORTED_R3B_RERUN = CLOSED_ABORTED
OLD_R3B_PARTIAL_OUTPUT = NON_CANONICAL
SUPERSEDED_R3_CAPACITY_OUTPUT = NON_CANONICAL
FIRST_DUAL_ARCH_RUN = INVALID_HARNESS_EMFILE
FIRST_DUAL_ARCH_RUN_CANONICAL = NO
NOFILE_FIX_APPLIED = YES
CANONICAL_RERUN_COMPLETED = YES

R3A_STATUS = REUSE_INFRA (tools/r3-p4-mock class A)
R3B_STATUS = PASS
R3B_AMD64_MAX_VALID_RPS = 75000
R3B_ARM64_MAX_VALID_RPS = 40000
R3B_EVIDENCE = .exyonq-local/tmp/r3-20260803/
R3C_STATUS = PASS
R3C_AMD64_SAFE_RPS = 62500
R3C_ARM64_SAFE_RPS = 31250
R3D_STATUS = PASS
R3D_TOPOLOGY_NEUTRALITY = PASS
R3E_STATUS = PASS_VALID
R3E_EXECUTED = YES
R3E_EXYONQ_REVISION = f0b2d67
R3E_AMD64_CLASSIFICATION = PARITY
R3E_ARM64_CLASSIFICATION = PARITY
R3E_OVERALL_CLASSIFICATION = PARITY
R3F_STATUS = PASS
PERFORMANCE_COMPILER_STATUS = ACCEPT
R3_HTML_UPDATED = YES
R3_PUBLIC_CLAIM_ALLOWED = YES
PRODUCT_CODE_CHANGED = NO

R3_NEXT_ACTION =
  STOP — R3 closed; owner may authorize R4 separately; do not auto-open R4/R5/GRC-P1
R4_OPENED = YES
R4_STATUS = COMPLETE
R4_RESULT = PASS
R4_CLOSED = YES
R4_READY_TO_OPEN = N/A
R5_OPENED = NO
GRC_P0_STATUS = COMPLETE
GRC_P1_STARTED = NO
GRC_R5_REMEDIATION_STATUS = COMPLETE
PRE_R5_GRC_READINESS = PASS
```

---

## 9b. GRC-R5-BLOCKER-REMEDIATION (2026-08-03)

```text
GRC_R5_REMEDIATION_STATUS = COMPLETE
GRC_P1_STARTED = NO
R4_STATUS = COMPLETE
R4_RESULT = PASS
R4_OPENED = YES
R4_CLOSED = YES
R5_STATUS = NOT_STARTED
R5_OPENED = NO
PRE_R5_GRC_READINESS = PASS
PRODUCTION_SIGNING_EXECUTED = NO
PUBLICATION_STATUS = FORBIDDEN
PUSH = NO
TAG = NO
RELEASE = NO
```

---

## 9c. R4 — Kubernetes baseline replay + OPEN-002 (2026-08-03)

```text
R4_STATUS = COMPLETE
R4_RESULT = PASS
R4_CLOSED = YES
R4_OPENED = YES

CURRENT_CANONICAL_BASELINE_AT_OPEN = 0fed18db79c386e5868718a99dbe25582c4b7adf
K8S_STRATEGY_BRANCH = strategy/exyonq-k8s-wedge
K8S_STRATEGY_HEAD = b70148e8edf0b9cfa043ecdf7031a1070fbc4c8d
P2A_REPLAY = 995634d (from ad33d762)
P2B_REPLAY = 4a1c2e9 (from b70148e)
OPEN_002_FIX = 1d815fe

P2B_OPEN_002_INITIAL = OPEN
OPEN_002_ROOT_CAUSE = Single/Multi gated on configured count not eligible count
OPEN_002_FIX_REQUIRED = YES
P2B_OPEN_002_FINAL = CLOSED

K8S_P2C_READY_TO_OPEN = YES
K8S_P2C_OPENED = NO
R5_STATUS = NOT_STARTED
R5_OPENED = NO
GO_CODE = NO
CARGO_TOML_CHANGED = NO
CARGO_LOCK_CHANGED = NO
DEPENDENCIES_ADDED = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION_STATUS = FORBIDDEN
```

R4 executed P2A/P2B replay onto `0fed18d`, closed OPEN-002 with eligible-count
Single gating (`1d815fe`), and productively accepted multi-endpoint WRR on the
canonical tip. P2C is ready to admit but **not** opened. R5 remains NOT_STARTED.

---

## 9b-cont. GRC-R5 remediation detail (unchanged)

Owner-authorized track closed **current** `R5_RELEASE_BLOCKER=YES` governance gaps
**before** opening R4. This is **not** R5 freeze/release execution.

| Phase | Result |
|-------|--------|
| GRC-R5A docs SoT (gitignore exceptions) | PASS — `docs/security/**` + `docs/governance/**` (+ releases) tracked |
| GRC-R5B security SoT | PASS — template/checklist/PMZ/waiver requirements |
| GRC-R5C release audit repair | PASS — selftest + dry-run-infra (`RELEASE_AUDIT_MISSING_PATHS=0`) |
| GRC-R5D integrity triad | PASS — policies + gates + fixtures; orphan `.pyc` removed |
| GRC-R5E PMZ + hooks + CI | PASS — canonical `scripts/security/scan-private-material.sh`; `.githooks`; CI wired |
| GRC-R5F signing governance | READY — design/verify path; **no** production signing |
| GRC-R5G exception register | PASS — `RUSTSEC-2026-0222` tracked (`VULNERABILITY_FIXED=NO`) |
| GRC-R5H evidence binder | PASS — `docs/governance/evidence/INDEX.md` |
| GRC-R5I enforcement matrix | PASS — `docs/governance/enforcement-matrix.md` |
| GRC-R5J pre-R5 readiness gate | PASS — `scripts/gates/pre-r5-grc-readiness-gate.sh` |

```text
DOCS_SECURITY_TRACKED = YES
DOCS_GOVERNANCE_TRACKED = YES
CURRENT_SOT_CONFLICT = NO
FINAL_R5_GRC_BLOCKERS = 0
```

---

## 10. GRC-P0 audit snapshot reconciliation (2026-08-03)

```text
GRC_P0_STATUS = COMPLETE
GRC_P0_ROLE = HISTORICAL_AUDIT_SNAPSHOT
GRC_P0_INSPECTION_HEAD = 8953b19d5709b4d34dda89caa9cef287fe11e708
GRC_P0_DOES_NOT_OVERRIDE_NEWER_CANONICAL_STATE = YES
GRC_P1_STARTED = NO
EXQ_TAXONOMY = PROVISIONAL (not canonical until GRC-P2 admission)
CONTROL_CATALOG_CREATED = NO
RISK_REGISTER_CREATED = NO
EXCEPTION_REGISTER_CREATED = NO
FINDING_REGISTER_CREATED = NO
EVIDENCE_REGISTER_CREATED = NO
```

Dimensions kept separate:

```text
PRODUCT_CORRECTNESS = PASS (BPC; no basic product defects)
PERFORMANCE_QUALIFICATION = R3 COMPLETE (RESULT=PASS; COMPETITIVE=PARITY; perf-compiler ACCEPT)
SECURITY_GOVERNANCE = READY_FOR_R5_INFRA (SoT + PMZ + audit infra; release audit for v0.4.3 not yet executed)
RELEASE_GOVERNANCE = READY_FOR_R5_INFRA (signing design READY; PRODUCTION_SIGNING_EXECUTED=NO; tag still FORBIDDEN)
EVIDENCE_GOVERNANCE = READY_FOR_R5_INFRA (binder present; local evidence remains LOCAL_ONLY)
SUPPLY_CHAIN_GOVERNANCE = PARTIAL (deny/audit/SBOM + verify tooling; production Cosign/OCI ceremony not executed)
```

### Particular rechecks vs current tree (post GRC-R5 remediation)

| # | Finding | Classification | Notes |
|---|---------|----------------|-------|
| 1 | `docs/security` missing | STALE_FIXED | Tracked SoT restored |
| 2 | `docs/governance` missing | STALE_FIXED | Tracked SoT restored |
| 3 | `/docs/` gitignore SoT | STALE_FIXED | `/docs/*` + exceptions for security/governance/releases |
| 4 | Integrity gates missing | STALE_FIXED | Triad + policies + fixtures restored |
| 5 | Orphan `__pycache__/*.pyc` | STALE_FIXED | Removed; sources tracked |
| 6 | `scripts/security/scan-private-material.sh` missing | STALE_FIXED | Canonical scanner restored |
| 7 | `.githooks` missing | STALE_FIXED | pre-commit + pre-push tracked; hooksPath coherent |
| 8 | PMZ scanner path drift | STALE_FIXED | CI + hooks + xtask ci wired |
| 9 | Release audit refs missing paths | STALE_FIXED | Template/checklist present; verifier selftest PASS |
| 10 | Cosign/OCI signing unwired | PARTIAL_BY_DESIGN | Governance READY; production ceremony not executed |
| 11 | Benchmark governance claims | STALE_FIXED | Integrity policy/gates restored |
| 12 | R3 resume vs NO_SMOKE contradiction | STALE_FIXED_OR_SUPERSEDED | Unchanged |
| 13 | Stale evidence/pass claims | STALE_FIXED | Evidence binder marks historical/non-canonical |
| 14 | Unmapped gates (no GRC catalog IDs) | REAL_BUT_OUT_OF_SCOPE | GRC-P2+; not opened here |

### P0 gap rollup after recheck (totals preserved; classifications updated)

```text
GRC_P0_FINDINGS_TOTAL = 28
GRC_CURRENT_FINDINGS = 27
GRC_STALE_FIXED = 1
GRC_STALE_SUPERSEDED = 0
GRC_HISTORICAL_ONLY = 0
GRC_UNVERIFIED = 0

# Severity among CURRENT only (P0 gap set after recheck):
GRC_CURRENT_CRITICAL = 6
GRC_CURRENT_HIGH = 7
GRC_CURRENT_MEDIUM = 9
GRC_CURRENT_LOW = 5

# Mapping notes:
# - P0 High#4 (R3 resume vs NO_SMOKE) → STALE_FIXED_OR_SUPERSEDED (not CURRENT)
# - P0 Critical#3 (no GRC registers) remains CURRENT Critical for GRC maturity,
#   but PRODUCT_BLOCKER=NO and is GRC_P1/P2 design-scoped (registers not invented here)
# - R3E EMFILE first attempt is R3 HISTORICAL_ONLY / NON_CANONICAL (not a P0 gap row)
```

### CURRENT R5/release-scoped GRC blockers (not auto-opened)

| GRC_FINDING | SEVERITY | CURRENT | PRODUCT_BLOCKER | R4_BLOCKER | R5_RELEASE_BLOCKER | GRC_P1_DESIGN_DEPENDENCY | REMEDIATION_REQUIRED_BEFORE_RELEASE |
|-------------|----------|---------|-----------------|------------|--------------------|--------------------------|-------------------------------------|
| Canonical `docs/security` audit SoT missing | Critical | NO (FIXED) | NO | NO | NO | — | CLOSED_BY_GRC_R5 |
| Release audit depends on missing files | Critical | NO (FIXED) | NO | NO | NO | — | CLOSED_BY_GRC_R5 |
| `/docs/` SoT conflict (ignore vs required paths) | Critical | NO (FIXED) | NO | NO | NO | — | CLOSED_BY_GRC_R5 |
| Integrity policy/gate triad absent | Critical | NO (FIXED) | NO | NO | NO | — | CLOSED_BY_GRC_R5 |
| PMZ canonical path + hooks + CI drift | Critical | NO (FIXED) | NO | NO | NO | — | CLOSED_BY_GRC_R5 |
| Cosign/OCI signing not productionized | Critical | PARTIAL | NO | NO | NO* | — | Design READY; ceremony deferred to R5 owner |
| Exception/waiver register informal | High | NO (FIXED) | NO | NO | NO | — | CLOSED_BY_GRC_R5 |
| Evidence provenance/binding weaknesses | High | NO (FIXED) | NO | NO | NO | — | CLOSED_BY_GRC_R5 |
| DB1 policy file absent (also under `/docs/`) | High | YES | NO | NO | NO* | YES | OWNER (arch policy SoT) |
| OPEN-002 / K8s | High | NO (FIXED_R4) | NO | NO | NO | NO | CLOSED_BY_R4 |
| No unified GRC registers | Critical@P0 | YES | NO | NO | NO | YES | GRC-P2+ (not invented now) |

\* Production Cosign/OCI **ceremony** remains owner-manual at R5; governance/verify path is READY (`PRODUCTION_SIGNING_EXECUTED=NO`).  
† OPEN-002 closed by R4 (`1d815fe`); R5 still requires separate owner open.  
\* Architecture DB1 policy under `/docs/` may still be local-only until separately force-tracked — not an R5 GRC release blocker after option-1 SoT.

### `/docs/` owner decision (GRC-R5A — option 1 implemented)

```text
DOCS_GITIGNORE_RULE = .gitignore:/docs/* + exceptions
TRACKED_DOCS_CANONICAL = docs/security/** + docs/governance/** + docs/releases/**
DOCS_SECURITY_TRACKED = YES
DOCS_GOVERNANCE_TRACKED = YES
CURRENT_SOT_CONFLICT = NO
```

### Integrity / PMZ / release-audit / signing status (tree truth post-remediation)

```text
INTEGRITY_GATES_CURRENT_STATUS = PASS (selftest + tree)
PMZ_CURRENT_STATUS = PASS (canonical scanner + hooks + CI)
RELEASE_AUDIT_CURRENT_STATUS = INFRA_PASS (system executable; v0.4.3 audit NOT_EXECUTED)
SIGNING_GOVERNANCE_CURRENT_STATUS = READY (verify+pubs; PRODUCTION_SIGNING_EXECUTED=NO)
PRE_R5_GRC_READINESS = PASS
```

### R2 dual-arch matrix (reconciled; R2E)

| CAPABILITY | DARWIN | AMD64 | ARM64 | OCI_AMD64 | OCI_ARM64 |
|------------|--------|-------|-------|-----------|-----------|
| workspace check | PASS | PASS | PASS | — | — |
| workspace tests | PASS | PASS | PASS | — | — |
| Clippy supported | PASS | PASS | PASS | — | — |
| static HTTP/1.1 | N/A | HISTORICAL C | HISTORICAL C | START+HTTP claim unverified in OCI harness | same |
| proxy | N/A | HISTORICAL C | HISTORICAL C | START+HTTP claim unverified in OCI harness | same |
| explicit cache | PASS | PASS | PASS | — | — |
| FastCGI | N/A | HISTORICAL B/C | HISTORICAL B/C | — | — |
| TLS | PASS ALPN | PASS | PASS | — | — |
| HTTP/2 | harness fixture | PASS | PASS | — | — |
| HTTP/3 | SKIP curl | PASS | PASS | — | — |
| reload/drain | harness Darwin | PASS | PASS | — | — |
| CLI/config | PASS | PASS | PASS | VERSION PASS | VERSION PASS |
| Wasm | PASS | PASS | PASS | — | — |
| SECINT | PASS | PASS | PASS | — | — |
| security scan | selftest | selftest PASS | selftest PASS | — | — |
| release build | PASS | PASS | PASS | — | — |
| container build | N/A | — | — | PASS | PASS |
| container version cell | N/A | — | — | HISTORICAL A NOT_GATE_CLOSING | HISTORICAL A NOT_GATE_CLOSING |

Product-qualified revision for Linux/OCI images remains `2f8e869…`.
R2E harness commit `2849d06…` is test-only (no OCI rebuild required).

Evidence (local only):

- R2 qual: `.exyonq-local/tmp/r2-qual-20260731/`
- R2A SECINT-001: `.exyonq-local/tmp/r2a-secint001-20260801/`
- R2B wasm: `.exyonq-local/tmp/r2b-wasm-20260801/`
- R2C clippy/advisory: `.exyonq-local/tmp/r2c-clippy-20260801/`
- R2D: `.exyonq-local/tmp/r2d-20260801/`
- R2E: `.exyonq-local/tmp/r2e-20260801/`
- R3 capacity/competitive (canonical): `.exyonq-local/tmp/r3-20260803/`
- R3E EMFILE attempt: remote `*-r3e-emfile-invalid/` — NON_CANONICAL / HISTORICAL_ONLY
- R3F reconciliation: `study/reports/R3F_RECONCILIATION.md`
- GRC-P0 snapshot: agent audit @ HEAD `8953b19` (historical); classifications in §10
