# Evidence index / binder (canonical)

```text
DOCUMENT = docs/governance/evidence/INDEX.md
ROLE = LIGHTWEIGHT_EVIDENCE_BINDER
COPIES_EXYONQ_LOCAL_INTO_GIT = NO
PROMOTES_LOCAL_TO_CANONICAL_BY_INDEXING = NO
```

This index **references** evidence. Local paths under `.exyonq-local/` remain
LOCAL_ONLY / NOT_RELEASE_CONTENT unless separately published under authorized
release process.

Vocabulary:

| Field | Meaning |
|-------|---------|
| CURRENT vs HISTORICAL | Whether the claim is the live authority for that gate |
| CANONICAL | May close a phase/gate when CURRENT |
| NON_CANONICAL | Diagnostic / superseded / invalid harness |
| NON_AUTHORITATIVE | Must not close gates (includes smoke) |

## Authoritative current bindings

| Claim / phase | Result | Product revision | Host / arch | Command / gate | Timestamp / run ID | Evidence location | CURRENT/HISTORICAL | Canonical? |
|---------------|--------|------------------|-------------|----------------|--------------------|-------------------|--------------------|------------|
| R5 version bump 0.4.2→0.4.3 | PASS | `e8fa4dca9336bb8293b56ca8ad47e24bfd4fd792` | Darwin worktree | Cargo/packaging/CHANGELOG/release allowlists | 2026-08-04 | git commit `e8fa4dc` | CURRENT | YES |
| R5 post-bump product tip (+ OCI E2E version label) | LOCKED | `7ca6d40c45cce16c0eb480c64e2a2b29b26ae3a6` | Darwin worktree | OCI E2E `EXYONQ_VERSION=0.4.3` | 2026-08-04 | git commit `7ca6d40` | CURRENT | YES |
| R5 post-bump Darwin gates | PASS | `7ca6d40…` | Darwin | fmt/check/clippy/tests + integrity + PMZ + pre-R5 + BPC | `r5-darwin-20260804T014828Z` | `.exyonq-local/release/r5-postbump-e8fa4dca9336/darwin/` | CURRENT | YES — LOCAL_ITERATION_ONLY |
| R5 post-bump Linux amd64 | PASS | `f90761c…` (parent of OCI label fix; product identity 0.4.3) | Netcup amd64 | full remote qual + version_coherence | `r5-linux-20260803T234634Z` | `.exyonq-local/release/r5-postbump-e8fa4dca9336/linux-amd64/` | CURRENT | YES |
| R5 post-bump Linux arm64 | PASS | `f90761c…` | Oracle arm64 | full remote qual + version_coherence | `r5-linux-20260803T234634Z` | `.exyonq-local/release/r5-postbump-e8fa4dca9336/linux-arm64/` | CURRENT | YES |
| R5 post-bump WRR protectors | PASS dual-arch | `f90761c…` | Netcup + Oracle | `p2b_linux_protectors` + perf-compiler ACCEPT | `r5-postbump-wrr-20260804T001112Z` | `.exyonq-local/release/k8s-p2b-protectors-r5-postbump-wrr-20260804T001112Z/` | CURRENT | YES |
| R5 post-bump OCI dual-arch + index | PASS_REAL_E2E + INDEX | `7ca6d40…` | Netcup+Oracle local OCI | `oci-runtime-e2e` + local multiarch index | `20260804T014828Z` | `.exyonq-local/release/r5-postbump-e8fa4dca9336/oci/` | CURRENT | YES (local; no GHCR) |
| R5 post-bump artifacts 0.4.3 | PASS | `7ca6d40…` | Netcup+Oracle WS6 | `exyonq-0.4.3-linux-{amd64,arm64}.tar.gz` + src + SBOM + SHA256SUMS | 2026-08-04 | `.exyonq-local/release/r5-postbump-e8fa4dca9336/artifacts/bundle/` | CURRENT | YES (local; unsigned) |
| R5 security audit v0.4.3 | PASS (doc) / CONDITIONAL ACCEPT compiler | Commit bind `e8fa4dc…` | Darwin + Linux gates | `docs/security/audit-v0.4.3.md` | 2026-08-04 | `docs/security/audit-v0.4.3.md` | CURRENT | YES — does not authorize tag/sign/publish |
| R5 pre-bump freeze (0.4.2-labeled binaries) | HISTORICAL | `e63325d…` | dual-arch | pre-bump R5 freeze | 2026-08-03 | `.exyonq-local/release/r5-freeze-6270968/` | HISTORICAL | NO — invalidated by version bump |
| R2 qualification close | PASS | `2f8e869…` (product); harness tip later | Darwin + Netcup amd64 + Oracle arm64 + OCI dual-arch | R2E workspace + KD3 harness | 2026-08-01 / r2e | `.exyonq-local/tmp/r2e-20260801/` | HISTORICAL | YES (pre-R4; **not** freeze tip) |
| NO-SMOKE / BPC | PASS | reconfirmed post-bump on `7ca6d40…` | Netcup + Oracle + Darwin | `no-smoke-as-proof-gate`, BPC gate | 2026-08-04 | `.exyonq-local/tmp/bpc-20260802/`; postbump Darwin/logs | CURRENT | YES |
| HTTP/3 POST-body fix | PASS dual-arch | `8b702a2` | amd64 + arm64 | product tests / BPC | 2026-08-02 | `.exyonq-local/tmp/bpc-20260802/` | HISTORICAL (retained in tip) | YES |
| OCI real E2E (pre-R5 NS5) | PASS_REAL_E2E | NS5 dual-arch (pre-freeze tip) | local OCI dual-arch | no-smoke NS5 | 2026-08-02 | `.exyonq-local/tmp/no-smoke-audit-20260802/` | HISTORICAL | NO — superseded by post-bump OCI @ `7ca6d40…` |
| R3A infra | PASS | `f0b2d67` | dual-arch | R3A | 2026-08-03 | `.exyonq-local/tmp/r3-20260803/` | CURRENT | YES |
| R3B capacity | PASS | `f0b2d67` | amd64/arm64 | R3B | 2026-08-03 | `.exyonq-local/tmp/r3-20260803/` | CURRENT | YES |
| R3C safe RPS | PASS | `f0b2d67` | amd64/arm64 | R3C | 2026-08-03 | `.exyonq-local/tmp/r3-20260803/` | CURRENT | YES |
| R3D topology neutrality | PASS | `f0b2d67` | dual-arch | R3D | 2026-08-03 | `.exyonq-local/tmp/r3-20260803/` | CURRENT | YES |
| R3E competitive | PASS_VALID / PARITY | `f0b2d67` | amd64+arm64 | R3E fixed-rate | 2026-08-03 | `.exyonq-local/tmp/r3-20260803/` | CURRENT | YES |
| R3F reconciliation | PASS (docs) | docs tip | — | R3F | 2026-08-03 | `study/reports/R3F_RECONCILIATION.md` (local study) | CURRENT | YES (documentary) |
| performance-compiler | ACCEPT | R3 close | — | exyonq-performance-compiler | 2026-08-03 | PROJECT_STATUS / agent record | CURRENT | YES |
| Integrity triad selftest | PASS | this remediation | Darwin local | `scripts/gates/*-integrity*` + data-provenance | GRC-R5D | scripts/gates | CURRENT | YES (gate machinery) |
| PMZ scanner | PASS | this remediation | Darwin local | `scan-private-material.sh --git-tree` | GRC-R5E | scripts/security | CURRENT | YES |
| Release-audit infra | PASS (system) | this remediation | Darwin local | `verify-release-audit.sh --dry-run-infra` | GRC-R5C | scripts + docs/security | CURRENT | YES — **not** v0.4.3 audit approval |

## Explicitly non-canonical / historical

| Item | Classification | Notes |
|------|----------------|-------|
| Owner-named open tip `6270968…` without post-open lint commits | HISTORICAL_OPEN_TIP | Superseded by freeze relock `e63325d…` (fmt/clippy/harness only; Cargo.toml/lock unchanged) |
| `PRODUCT_QUALIFIED_REVISION=2f8e869…` as freeze proof | HISTORICAL_PRE_R4 | Must not qualify current freeze tip |
| Historical WRR proof @ `b70148e` | HISTORICAL_STRATEGY | Not used; tip-bound `r5-wrr-20260803T162708Z` is CURRENT |
| R3E EMFILE first dual-arch run | HISTORICAL_NON_CANONICAL | Invalid harness; superseded by NOFILE fix + canonical rerun |
| Aborted rsync evidence pull | NON_RESULT | Abort does not affect R3 result |
| Old smoke evidence / `*SMOKE*` reports | NON_AUTHORITATIVE | Rule 127 — must not close gates |
| R2D OCI version-only cells | HISTORICAL / NOT_GATE_CLOSING | Class A version cells only |
| GRC-P0 audit snapshot @ `8953b19` | HISTORICAL_AUDIT_SNAPSHOT | Does not override newer product/R3 state |
| R5 amd64 initial PMZ/GRC FAIL | HISTORICAL_HARNESS_DEFECT | `.cursor`/`.git` rsync; remediate PASS is CURRENT |
| R5 arm64 initial FAIL (disk full) | HISTORICAL_INFRASTRUCTURE | Rebind PASS is CURRENT |

## Integrity invariants for this binder

```text
CURRENT_PASS_WITHOUT_EVIDENCE_BINDING = 0   # for claims listed above
HISTORICAL_AS_CURRENT = 0
NONCANONICAL_AS_CANONICAL = 0
```
