# ExyonQ security audit — v0.9.3

> **Re-audited for the 0.9.3 patch (2026-11-25), now committed.** This seal
> describes the **74 tracked + 10 untracked = 84-entry** 0.9.3 patch audited on
> HEAD `0f58cae` and committed as `C d145976`, including issue #109
> (FastCGI/PHP-FPM), the ACME-per-server/H3-SNI increment and the strict
> fingerprint binding in `scripts/verify-release-audit.sh`. The seal binds the canonical tracked diff
> and a sorted manifest of every untracked file (path + content SHA-256); the
> combined `WORKTREE_FINGERPRINT_SHA256` is `89788d3c…` / **84** entries. The verifier fails with
> `CONTROL=worktree_fingerprint_mismatch` when either component changes; the old
> 48-file seal is retired and retained only as history.

| Field | Value |
| --- | --- |
| **Version** | 0.9.3 |
| **Commit** | `d145976b486c243c8e8c8970cd0f45ec6a425977` |
| **Date** | 2026-11-25 |
| **Previous audit** | [`audit-v0.9.2.md`](audit-v0.9.2.md) |

> **Snapshot note.** The tracked diff is a moving target in this workspace: after the
audit runs, `CHANGELOG.md`, `docs/release/v0.9.2.md` and
>`docs/releases/0.9.2-release-evidence.md` were edited outside this increment (0.9.3
>preparation and the Docker Hub mirror record), and the issue #109/ACME/H3 work plus
>the release tooling landed afterwards. The current snapshot was taken on 2026-11-25
>**after every tracked edit**: 74 modified tracked files and 10 untracked files (8
>`git status` entries; `supply-chain/` contributes three files). Every untracked
>path and content hash is in the sorted manifest below, not just the count. The current-tree
>re-audit below was run on that snapshot. The 2026-11-20/23 rows are retained as
>history and are **not** presented as results for this tree. Nothing was reverted and
>no file outside this seal's scope was touched.
>
> **What the Commit field names here.** The audited state was HEAD `0f58cae`
> **plus the 0.9.3 patch** (74 modified tracked files and 10 untracked files,
> including issue #109 and ACME/H3), bound by the worktree fingerprint below.
> That patch is committed as **C**
> `d145976b486c243c8e8c8970cd0f45ec6a425977`, and this document is committed as
> the audit-only seal **S** `ffbc2005d54f6d9e99e295f2570e9b240697406b`, whose
> first parent is C and whose only change is this file. C and S were published
> to `main` through merge `1b92e6a4199b50e5cffa80ebcb68583a13fdaaf1` (PR #110).
> `v0.9.3` is not tagged yet: the tag belongs on S, not on C.

```text
AUDITED_STATE      = C d145976b486c243c8e8c8970cd0f45ec6a425977 (the 0.9.3 patch commit)
                     audited on HEAD 0f58caea08899744ec6f34a5671b41fac18a6ea6 as 74 tracked + 10 untracked = 84 entries (includes #109 and ACME/H3)
WORKTREE_INTEGRITY_BEGIN
WORKTREE_DIFF_SHA256 = dbbab9bb532eda55883e72acfcd151654e2c78a587fba25d2a8bd87172cc715f (canonical tracked `git diff`; snapshot 2026-11-25, after the #109 FastCGI fix, the ACME/H3 increment, the strict fingerprint binding in `scripts/verify-release-audit.sh`, and all other tracked edits)
WORKTREE_DIFF_FILES  = 74 modified (git diff --name-only)
WORKTREE_UNTRACKED_SHA256 = 242feab16b15098bd2b46b3500d5b7a2720a1ce89137f7f2d1a4c798c961a494 (SHA-256 of the sorted path<TAB>content-SHA-256 manifest below)
WORKTREE_UNTRACKED_FILES = 10 untracked files
WORKTREE_FINGERPRINT_SHA256 = 89788d3c960d665e74afe43d84c2241d600af3462d6c64d2cad0f3750561ff94 (combined fingerprint over the tracked diff hash/count and the untracked manifest hash/count)
WORKTREE_FINGERPRINT_FILES = 84 total entries (74 tracked + 10 untracked)
WORKTREE_UNTRACKED_MANIFEST_BEGIN
docs/architecture/fastcgi-php-fpm-operations.md	0c2c2aef1bcd354447684a7a9dad1fc0c03b227be6d780987e3058fb373b2545
docs/release/v0.9.3.md	43c28b5ad9f5192047e8387357e45c9c092c64ccbd8330b82a7be55205ad022e
docs/security/audit-v0.9.3.md	e1d0a6bd0b1e93f7862175ef6d31d6a302a61a9ecfffac8ab228156a72de4c21
scripts/gates/linux-cross-check.sh	31263f4e94eacd0f352841725cb5fc45f6840375fb1c946cd1a1791ee13eeeac
scripts/gates/portable-shell-gate.sh	df809a7045bc0bf62356305b46129bd6ae11b72fe8557cc8c63049d3d57be020
scripts/gates/tag-signature-gate.sh	ff83776280d5e417427112c75f1c73bc27e8c9eca43cdf829efbd3440ecb6529
scripts/release/mirror-to-dockerhub.sh	49ee5bd96191eae91bb986e8382a8ab6bfe70692cdf97b9898974aff6398db20
supply-chain/audits.toml	628720e4cf05595b314bd06d3bb73e6434e3d3be10f1a394dfbbfe6ea248403c
supply-chain/config.toml	66160f8b802d5200115116aaa1a987348e8a6fbe88919858f51615a2343bcc16
supply-chain/imports.lock	ff01c4f0f1a725196404edd25fbab0b4ad073de1264fa124b92e2344ec39eb7a
WORKTREE_UNTRACKED_MANIFEST_END
WORKTREE_INTEGRITY_END
WORKTREE_UNTRACKED   = 10 files / 8 status entries (docs/architecture/fastcgi-php-fpm-operations.md,
                           docs/release/v0.9.3.md, docs/security/audit-v0.9.3.md,
                           scripts/gates/linux-cross-check.sh,
                           scripts/gates/portable-shell-gate.sh,
                           scripts/gates/tag-signature-gate.sh,
                           scripts/release/mirror-to-dockerhub.sh,
                           supply-chain/config.toml, supply-chain/audits.toml,
                           supply-chain/imports.lock — the vet store created here)
RELEASE_BINARY     = target/release/exyonq sha256 720d5a6efe78132998454b12b1fa5e9dd87207d4dd74a28d55bae4ab1f755bc9 (rebuilt 2026-11-25 after #109/ACME; local smoke DEVELOPMENT_AID PASS=5 FAIL=0, not E2E)
BINARY_VERSION     = exyonq 0.9.3 / product_version=0.9.3 / artifact_version=0.9.3 / source_revision=0f58caea… / source_tag= (empty, untagged tree)
SEAL_COMMIT        = S ffbc2005d54f6d9e99e295f2570e9b240697406b (audit-only; first parent C; changes only this file)
PUBLISHED_IN       = 1b92e6a4199b50e5cffa80ebcb68583a13fdaaf1 (merge of PR #110 into main; no tag)
TAG                = NONE (not created; the tag ceremony is still owner-gated)
```

**Seal construction (2026-11-25).** `WORKTREE_DIFF_SHA256` is the SHA-256 of
the canonical tracked `git diff`; `WORKTREE_UNTRACKED_SHA256` is the SHA-256 of
the sorted manifest of **every** untracked file (`path<TAB>content-SHA-256`),
and `WORKTREE_FINGERPRINT_SHA256` hashes both components and their counts.
Adding, deleting or editing any untracked file changes the manifest, not just a
count. The verifier rehashes the embedded manifest against
`WORKTREE_UNTRACKED_SHA256/FILES` and reports the changed paths. The seal's own
integrity block and citations of the seal-managed values are normalized before
hashing, because a document cannot contain its own final hash; full and
8-hex-prefix citations use different tokens, so replacing one form with the
other changes the hash. A citation check also rejects any
`WORKTREE_FINGERPRINT_SHA256` line that names a different value. Every other
byte is covered. There is no clean-tree, legacy or count-only PASS path.
`--dry-run-infra` reports `READY` and never emits `RELEASE_AUDIT_GATE=PASS`;
even with `EXYONQ_AUDIT_EXPECT_VERSION` set it checks presence/version shape only.

## Summary

0.9.3 is a patch on the 0.9.2 line: it moves the `wasmtime` pin to `=49.0.6`
(RUSTSEC-2026-0566 … -0572), carries the already-reviewed access-event (#84) and
healthcheck (#81) fixes, bumps the workspace version and the release-tooling
version allowlist, and adds no new dependency, crate, config surface, protocol
path or route. The audit was executed on the real tree and the real release
binary; Veyra `standard`, Rust Audit, CodeMap, the private-material and
publication-privacy gates, `cargo audit`/`cargo deny`, the local proxied smoke
and the full workspace suite are recorded below with their commands and exit
codes. **The current 84-entry tree was re-audited on 2026-11-25** (Veyra
`ACCEPTED`, 2346 tests, 76.00 % lines; Rust Audit 28 critical/118 high with no
critical in the #109/ACME files; CodeMap clean; rebuilt binary `720d5a6e…`),
and the seal is bound to `WORKTREE_FINGERPRINT_SHA256 = 89788d3c…` / **84 entries (74 tracked + 10 untracked)**. The
2026-11-20/23 rows below are retained as history, not as current results.

The extra Rust Audit legs were executed for this seal with real verdicts:
`--supply-chain` (cargo audit 0 advisories, deny ok, guarddog clean, **cargo vet
pass with 464 baseline exemptions**), `scan --online`, `--miri` (199 tests under
Miri with isolation disabled) and `--deep` (coverage pass, **machete: no unused
dependencies**, **mutation: 31 mutants, 23 caught / 0 missed / 8 unviable** over
the two addon crates). The 12 survivors of the earlier run were closed by seven
contract tests (F-01, see *Findings closed in this seal*); no finding stays open.
The `--deep` leg itself still returns RC **1**, and only through the auditor's
severity gate over restriction-lint diagnostics the repository never enables
(*Deep leg: the severity gate*, an owner decision); this seal does not claim
RC 0 for that leg.

**No open blocker and no waiver.** Everything Rust Audit classified as critical
was reviewed one by one and is either a deliberate must-fail fixture or a
classifier false positive on protocol identifiers/checksums; the detail is in
*Reviewed and dismissed*. The audit found **two real test-isolation defects**
in this tree; both were fixed here with the shipped tests, not waived (see
*Findings (closed this release)*).

## Current-tree re-audit (2026-11-25)

This section is the evidence for the 84-entry snapshot above and supersedes the
2026-11-20/23 rows for step 7. All commands ran on macOS arm64 in this
workspace; nothing was committed, pushed, tagged or published.

| Tool | Command | Result |
| --- | --- | --- |
| Veyra Proof 0.7.4 | `veyra audit . --profile standard --yes` | exit **0**, `Decision: ACCEPTED — All required profile tools executed without blocking findings`. OK: rustfmt, cargo check, clippy, cargo test (**2346 passed / 0 failed**; nextest `2346 tests run: 2346 passed, 1 skipped`), cargo-audit (**no advisories**), veyra-anti-theater (no blocking finding; 392 informational VT006), cargo-llvm-cov (**76.00 % lines**, 35 758/149 015), cargo-deny, cargo-machete (no unused dependencies). **NOT EXECUTED as a pass (2026-11-25):** `cargo-geiger` **INCONCLUSIVE** (54 coverage-warning runs; unsafe counts are a lower bound); dylint (no lint configuration), cargo-semver-checks (no baseline) and veyra-e2e (no `[[e2e.scenarios]]`) were **SKIPPED/NOT EXECUTED**. None counts as PASS. Evidence `.veyra/evidence/20261125T215432Z/report.md`; log `.exyonq-local/tmp/issue109/veyra-reaudit-20261125.log`. |
| Rust Audit 0.2.9 | `RUST_AI_AUDIT=… python3.13 …/rust_audit.py scan . --format md --out report --fail-on high --exclude '.exyonq-local/**,target/**,.veyra/**,report/**,scripts/gates/fixtures/**'` | **NOT A PASS — exit 1 by the severity gate (2026-11-25):** **28 critical / 118 high / 985 medium / 422 low / 7 info**. Report `report/rust-audit-20261125-225601.md`; not-executed list `report/not-executed-2026-11-25.md`. The scan excluded local copies/build output and the must-fail fixtures only. **No critical is in a file added or touched by #109 or ACME/H3**; the 28 criticals are the same 27 R12 secret-like-literal false positives (cipher names, header names, checksums, redaction needles) plus 1 R18d offline-table false positive, adjudicated below. RC 1 is the auditor's `--fail-on high` gate, not a product-test failure; it is not reported as green. |
| CodeMap 0.2.38 | `bash scripts/ops/codemap-reindex.sh` + `codemap status --root .` + `bash scripts/ops/codemap-lint-gate.sh` | `CODEMAP_INDEX=OK`; **628 files, 11 398 nodes, 11 845 edges, 6 gaps**, `head=0f58cae…`, `stale=false`, `frescura: limpio (0 ficheros sucios)`, `páginas: 4/4`; `CODEMAP_LINT_GATE=PASS (18 checks triaged; 0 warnings; lock_out_of_sync excluded)`. Advisory pointers only. |
| Release binary | `cargo build --release --locked --bin exyonq` | RC **0** (2 m 34 s); `target/release/exyonq` sha256 **720d5a6e…**; `--version` = `exyonq 0.9.3`, `source_revision=0f58caea…`, `source_tag=` (empty, untagged). Proxied smoke (DEVELOPMENT_AID) against the rebuilt binary: `proxy-smoke: PASS=5 FAIL=0`, RC 0 (GET/query/64 KiB body/404 isolation; WAF 403 informational). Log `.exyonq-local/logs/proxy-smoke-0.9.3-20261125.log`. |
| Fingerprint binding | `bash scripts/verify-release-audit.sh --selftest` and `… 0.9.3` | selftest **PASS**, including `clean_tree_unverifiable`, `worktree_diff_mismatch`, `worktree_diff_match`, `legacy_no_fingerprint`, `untracked_content_match`, `untracked_content_mismatch`, `untracked_content_restored`, `untracked_add_delete_base`, `untracked_added`, `untracked_add_removed`, `untracked_deleted`, `untracked_delete_restored` and `citation_mismatch`; current seal → `RELEASE_AUDIT_GATE=PASS`, RC **0**. A deliberately modified tracked file and a separately modified untracked file each produced `CONTROL=worktree_fingerprint_mismatch` / RC **1**; both were restored byte-for-byte and returned PASS. The untracked failure named the changed path. |
| Documentation/evidence gates | `bash scripts/gates/q13-docs-audit.sh`, `bash scripts/gates/real-evidence-gate.sh --files docs/release/v0.9.3.md docs/security/audit-v0.9.3.md`, `git diff --check` | all RC **0** (recorded in the final verification section below). |

The audit runs above were executed before the final strictness edits in
`scripts/verify-release-audit.sh` (the clean-tree and legacy-seal branches now
fail explicitly, and the selftest covers them): those edits are shell/tooling
only, and no Rust source, lockfile, dependency or config changed after the
audit. The fingerprint below covers the final bytes, including those edits.

**Current-tree Rust Audit findings.** The 28 criticals are adjudicated in
*Current-tree findings review (2026-11-25)* below. The 118 highs are reviewed by
rule: the same R09/R03/R11/R20/R16/R21 classes as the historical run, plus one
R18a unresolved-`use` heuristic on a FastCGI integration test that compiles
(`crates/exyonq-mod-fastcgi/tests/unix_transport_integration.rs:18`) and
pre-existing R09 `expect` sites in `crates/exyonq-http3-provider-quiche`.
No real defect was found in the #109 or ACME/H3 increments; the auditor's RC 1
is recorded, not waived.

**NOT EXECUTED on the 84-entry tree (2026-11-25), never as passes:**
- Rust Audit `--supply-chain`, `--miri`, `--deep` and `scan --online`: not
  rerun on this tree; the 2026-11-20 results are historical.
- Fuzz campaigns: no campaign was launched on this tree.
- CodeQL and the hosted security workflows: NOT EXECUTED because nothing was
  pushed.
- Native Linux amd64/arm64 builds and the authoritative `proxy-e2e.sh`:
  NOT EXECUTED because this host is macOS and the suite is Linux-only.
- Docker images, Scout and cosign: NOT EXECUTED because nothing was tagged,
  pushed or published and cosign is absent on this host.

The current-tree replacements are Veyra's cargo-audit/cargo-deny and the local
smoke DEVELOPMENT_AID above; neither substitutes for those gates.

## Release gates (historical 2026-11-20 snapshot)

These rows describe the earlier 48-file snapshot. The current-tree checks are in
*Current-tree re-audit (2026-11-25)* above; the hosted gates below remain NOT
EXECUTED because nothing was pushed.

| Gate | Status | Notes |
| --- | --- | --- |
| CI (`ci.yml`) | NOT EXECUTED (no push) | Local equivalent `cargo run -p xtask -- ci` → **RC 0**, **2335 passed / 0 failed**, 0 lines of `test result: FAILED`; log `.exyonq-local/logs/xtask-ci-ng21job-20261120.log`. The workflow itself cannot run without a push, which is owner-gated. |
| Security (`security.yml`) | NOT EXECUTED (no push) | Locally: `cargo audit` RC 0 with **no warnings** after T-03 (the former RUSTSEC-2026-0486 `rustix` warning is gone), `cargo deny check advisories` → `advisories ok`, `cargo deny check bans sources` → `bans ok, sources ok`, `osv-scanner` on the product lock → 0 findings, `scan-private-material.sh --publication` → `PRIVATE_MATERIAL_FINDINGS=0` / `PRIVATE_MATERIAL_GATE=PASS` (`PUBLICATION_FILES=1429` on this tree: tracked files plus the 9 untracked ones, this seal and the four new gate/release scripts and the three `supply-chain/` files included), `publication-privacy-gate.sh` → `PUBLICATION_PRIVACY_FINDINGS=0` / `PASS`. |
| Nightly fuzz | pass (local campaigns, same tree) | Two `main` campaigns, both 0 crashes: `main-20261120` (`scripts/fuzz/p15-ws2-run.sh`, 8 targets, 45–90 s each, 510 s of fuzzing): 8/8 **PASS**, **10 358 218 runs** (sum of the eight `*.main.log` under `docs/operations/evidence/p1.5-ws2/main-20261120/`), `CRASHES=0 HANGS=0 OOM=0`; `main-final-20261120`: `proxy_headers`/`nginx_migrate`/`htaccess_parse`, **3 376 097 runs** (sum of its three logs under `docs/operations/evidence/p1.5-ws2/main-final-20261120/`), 0 crashes. The CI-wired runner (`scripts/security-fuzz.sh --mode main --budget 60`, RUN_ID `main-20261120T214814Z`, `.exyonq-local/evidence/fuzz-main-20261120/`) adds 8/8 **PASS** with **11 823 574 runs** and 0 crashes on its own campaign. `--mode probe`: 32 000 runs, 0 crashes (`probe-simplify-20261120`; `probe-final-20261120` holds two 32 000-run passes). `fuzz/artifacts/` holds no files in every campaign. The nightly workflow itself is NOT EXECUTED (no CI run). |
| Functional (old F1–F13) | re-scoped | The F1–F13 harness does not exist in this tree (release checklist §4). Coverage used instead: the full workspace suite above plus the local proxied smoke under *Wire and route parity*. |
| WASM host tests | pass | `cargo test -p exyonq-wasm-host --locked` → **10 passed** (7 lib + 2 `independent_add_oracle` + 1 `plugin_load`), 0 failed. |
| Benchmark Tier A | NOT EXECUTED | No benchmark campaign ran in this increment; no performance claim is made. |
| Release workflow, images, Docker Hub, Scout, cosign | NOT EXECUTED | Nothing was tagged or pushed. These run at the tag and are recorded in `docs/release/v0.9.3.md` after publication. |

## Automated analysis (historical 2026-11-20/23 snapshot)

These rows are retained for provenance. They describe the earlier 48-file tree
and are **not** results for the current 84-entry snapshot; the current-tree table
above is the evidence for step 7.

| Tool | Command | Result |
| --- | --- | --- |
| Veyra Proof 0.7.4 | `veyra audit . --profile standard --yes` | exit **0**, `Decision: ACCEPTED — All required profile tools executed without blocking findings`. OK: rustfmt, cargo check, clippy, cargo test, cargo-audit (no advisories), veyra-anti-theater (no finding above informational), cargo-nextest (**2322 passed / 2322 run, 1 skipped**), cargo-llvm-cov (**76.03 % lines**, 35 348/147 452), cargo-deny, cargo-machete (no unused dependencies). Not passes: `cargo-geiger` **INCONCLUSIVE** (coverage warnings in 54 runs; unsafe counts are a lower bound) and **SKIPPED** `dylint` (no lint configuration), `cargo-semver-checks` (unpublished crates, no baseline) and `veyra-e2e` (no `[[e2e.scenarios]]`). `No project manifest or lockfile changes detected`. Evidence `.veyra/evidence/20261120T072751Z/report.md` (+ `report.json`, `manifest.json`, `hashes/`), log `.exyonq-local/logs/veyra-standard-093-final-20261120.log`; an earlier `standard` run on the pre-fix tree (`20261120T023124Z`) is superseded by this one. |
| Rust Audit (`.exyonq-local/tools/rust-ai-audit`, tool version 0.1.0) | `python3.13 scripts/rust_audit.py scan .exyonq-local/clean-093 --format md --out report --fail-on high` | exit **1** by `--fail-on high`: **33 critical / 411 high / 773 medium / 414 low / 7 info**. Report `report/rust-audit-20261120-033414.md`; not-executed list `report/not-executed-2026-11-20.md`. Run against a clean export (`.exyonq-local/clean-093`, local-only) because the first run on the full tree counted the `.exyonq-local/` copies of the repository (282 critical / 4552 high) and is not evidence; the tool version here is 0.1.0, not the 0.2.9 used for 0.9.2, so the severity totals are not comparable across releases. |
| Rust Audit `--supply-chain` | `python3.13 .exyonq-local/tools/rust-ai-audit/scripts/rust_audit.py cargo . --supply-chain --format md,json --out report/sc2 --budget 900` (the auditor is the local tool checkout, not a repo script) | exit **1** (severity gate). Legs: `cargo fmt` **pass**; `cargo clippy` findings (idiom lints, see *Reviewed and dismissed*); `cargo test` **pass** (82 s, cached); `cargo audit` **pass** (0 advisories, 2 s); `cargo deny check` **pass** (2 s); **`cargo vet check --output-format=json` pass** (1 s) — see *Vet store* below; `guarddog crates verify Cargo.lock` **pass** (85 s, `No risks detected`); `osv-scanner scan source -r .` **findings, exit 1** on untracked/local lockfiles only (see *OSV scope*). `cargo unmaintained` and `cargo supply-chain list-owners` remain **exit 101** — `error: no such command` (not installed; `cargo machete` and `cargo deny` cover the same ground). Report `report/sc2/rust-audit-20261120-193650.json`. |
| Rust Audit `scan --online` | `python3.13 scripts/rust_audit.py scan .exyonq-local/clean-093 --online --format md,json --out report/online --fail-on high` | exit **1**: **96 critical / 422 high / 773 medium / 414 low / 7 info**. The 68 extra criticals over the offline run are `R18d` "Crate `exyonq-…` does not exist on crates.io" (63 findings over 40 workspace members, all `publish = false`); `R18c` flags `ring`, `redis`, `rcgen`, `nix`, `perf` as non-existent although every one resolves from crates.io in `Cargo.lock`; `R19` returns the same two OSV advisories as the supply-chain leg; `R18a` reports 5 unresolved `use` paths that are test-module imports; the "impossible version requirement `2` for `thiserror` / `6` for `dashmap`" claims are false positives — crates.io reports `thiserror` 2.0.21 and the lock resolves `dashmap 6.2.1`. Report `report/online/rust-audit-20261120-072705.md`. |
| Rust Audit `--miri` | `python3.13 scripts/rust_audit.py cargo . --miri --format md,json --out report/miri --budget 1500` | exit **1**: `cargo +nightly miri test` **fails after 517 s** with `error: unsupported operation: 'open' not available when isolation is enabled` at `cli/exyonq/tests/version_metadata.rs` — the test spawns `exyonq --version`, so default-isolation Miri cannot execute this suite at all. With Miri's documented flag: `MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test -p exyonq-config-ir --lib` → **199 passed / 0 failed** in 477.61 s, RC **0**, **no UB reported**. Workspace-wide Miri is environment-blocked (process-spawning tests), not a project defect. Report `report/miri/rust-audit-20261120-074055.md`. |
| Rust Audit `--deep` | `RUST_AUDIT_MUTANTS_PACKAGES=exyonq-addon-sdk,exyonq-addon-api RUST_AUDIT_MUTANTS_TIMEOUT=30 RUST_AUDIT_MUTANTS_JOBS=8 python3.13 .exyonq-local/tools/rust-ai-audit/scripts/rust_audit.py cargo . --deep --format md,json --out report/deep4 --budget 2400` | exit **1** — the severity gate (`--fail-on high`, the tool default) trips on **5021 `high`** findings that are all restriction-lint diagnostics: `clippy::expect_used` 2586, `clippy::unwrap_used` 2270, `clippy::panic` 165 (plus 4014 `medium`: `indexing_slicing` 2204, `arithmetic_side_effects` 1810). **Every other leg passes**: fmt, test, llvm-cov, mutants (23 caught / 0 missed) and machete. The project's own clippy policy is green (`cargo clippy --workspace --all-targets --locked -- -D warnings` → RC 0); see *Deep leg: the severity gate* below. `cargo fmt` **pass** (2.0 s); `cargo test` **pass** (204.9 s); **`cargo llvm-cov` pass** (335.0 s; Veyra measures **76.03 % lines**); **`cargo mutants` exit 0** (833.5 s): `cargo mutants --no-shuffle -p exyonq-addon-sdk -p exyonq-addon-api --timeout 30 --jobs 8` → **31 mutants tested in 14 min: 23 caught, 0 missed, 8 unviable, 0 timeout**; **`cargo machete` pass (exit 0, 0.5 s)**, printing `cargo-machete didn't find any unused dependencies in this directory`; `cargo clippy` exits **0** in 25.1 s. Scope is the two addon packages only and is never presented as workspace-wide mutation coverage; the 12 survivors of the earlier `report/deep2` leg were closed by contract tests (see *Findings closed in this seal*). Reports `report/deep4/rust-audit-20261120-214001.{md,json}` (current tree, after the `dns_refresh` test-isolation fix). |
| CodeMap 0.2.38 (leg 3) | `codemap index --root . --json` + `codemap status --root . --json` | `ok:true`, **628 files, 55 crates, 11 321 nodes, 12 333 edges**, `gaps` 400 (truncated), `head=0f58caea08899744ec6f34a5671b41fac18a6ea6`, **`stale:false`**, `graph_hash=1edee56089581789`. Advisory pointers only; claims below were checked against the source. |
| Supply chain | `cargo audit`, `cargo deny check advisories`, `cargo deny check bans sources`, `osv-scanner scan source --lockfile=Cargo.lock` | `cargo audit` RC 0 with **no warnings** after T-03; `advisories ok, bans ok, sources ok`; OSV on the product lock → **0 findings**. |
| Release pre-flight | `cargo run -p xtask -- security pre-release --version 0.9.3` | **RC 0**; within it `cargo deny check` → `advisories ok, bans ok, licenses ok, sources ok`, security integration tests **11 passed / 0 failed**, wasm-host integration **3 passed / 0 failed**, then `RELEASE_AUDIT_GATE=PASS`. Log `.exyonq-local/logs/xtask-pre-release-093-20261120.log`. |
| Integrity, privacy, unsafe ratchet, waivers | `project-integrity-gate.sh --tree .`, `unsafe-allowlist-gate.sh --tree .`, `waiver-expiry-gate.sh --check`, `publication-privacy-gate.sh` | All PASS; `UNSAFE_ALLOWLIST_GATE=PASS` (`BASELINE_STALE_ENTRIES=2`), `WAIVER_EXPIRY_ENFORCEMENT=PASS` (`CURRENT_RELEASE_WAIVERS_UNREGISTERED=0`), `PUBLICATION_PRIVACY_FINDINGS=0`. |

**OSV scope, verified before calling them findings.** The two advisories are real
but informational and out of the product graph: `RUSTSEC-2026-0543` (*chrono is
unmaintained*, published 2026-09-14) affects `chrono 0.4.45`, and
`RUSTSEC-2025-0134` (*rustls-pemfile is unmaintained*, published 2025-11-28)
affects `rustls-pemfile 2.2.0`. Both appear **only** in the untracked benchmark
tool lockfiles `benchmarks/tools/http-loadgen/Cargo.lock` and
`benchmarks/tools/http3-loadgen/Cargo.lock` (`git ls-files` on those directories
is empty), never in the workspace `Cargo.lock` (`cargo tree -i chrono` and
`cargo tree -i rustls-pemfile` both answer `did not match any packages`).
Product-scoped re-run: `osv-scanner scan source --lockfile=Cargo.lock` (519
packages) → **0 findings** after T-03; `cargo audit` → RC 0 with **no warnings**.

**Vet store, created and exercised.** `cargo vet init` (cargo-vet 0.10.2) wrote the
store the project expects at the repository root: `supply-chain/config.toml`
(version `0.10`, **464 `[[exemptions.*]]` baseline exemptions** at the current
locked versions), `supply-chain/audits.toml` (empty) and
`supply-chain/imports.lock` (empty). `cargo vet check` then reports
**`Vetting Succeeded (464 exempted)`**, exit **0**; the JSON interface gives
`conclusion: success`, `vetted_fully: 0`, `vetted_partially: 0`,
`vetted_with_exemptions: 464`. This is a real verdict with an explicit meaning:
the pinned dependency set is trusted through init-generated exemptions, **not**
through imported audits of the dependency sources, so any version change makes
the check fail until that crate is audited or exempted. Committing
`supply-chain/` and deciding an imports policy (for example importing the Mozilla
or Bytecode Alliance audit sets with `cargo vet import`) is an owner decision;
the exact state is reproducible with `cargo vet check` and
`cargo vet check --output-format=json` (log
`.exyonq-local/logs/cargo-vet-check-20261120.json`).

**Mutation scope and tooling fixes.** Mutation testing is scoped and budgeted
on purpose: `cargo mutants --no-shuffle -p exyonq-addon-sdk -p exyonq-addon-api
--timeout 30 --jobs 8` (the two addon crates), 31 mutants in 15 min, 0 timeouts.
The whole-workspace run stays out of scope because it exceeds any per-tool budget
(measured: SIGKILL at 1800 s). The local Rust Audit checkout was corrected so the
leg produces verdicts instead of tool errors: `cargo machete --json` is now probed
and falls back to the documented text interface on 0.9.2 (which has no `--json`),
and the mutants invocation accepts `RUST_AUDIT_MUTANTS_PACKAGES`,
`RUST_AUDIT_MUTANTS_TIMEOUT` and `RUST_AUDIT_MUTANTS_JOBS`. Those edits live in
`.exyonq-local/tools/rust-ai-audit` (local-only, not part of the repository or its
release artifacts); the repo-side commands above reproduce the same verdicts.

**Increments after the recorded deep run (2026-11-20).** Three product increments
landed after the last `--deep` evidence (`report/deep4`, 21:40), all local and
all three green in the full `xtask ci` (RC 0, **2335 passed / 0 failed**):

1. **H2 liveness deadline.** The HTTP/2
builder now arms h2's PING liveness probe with the committed
`[tuning].keepalive_timeout_ms` deadline (`core/src/server/mod.rs`), so an H2 peer
that completes a request and stops answering no longer holds the socket forever;
two real h2c tests in `tests/integration/ng21_h2_header_limits.rs` cover it
(5/5 pass, ~1.5 s, five consecutive runs) with the negative control failing at
5.04 s when the wiring is removed. `cargo fmt` and `cargo clippy --workspace
--all-targets --locked -- -D warnings` are RC 0 and `cargo run -p xtask -- ci`
reports **RC 0 with 2335 passed / 0 failed**. The deep leg was **not** re-run on
this exact tree (the mutation scope, the two addon crates, is untouched), so its
numbers stay those of `report/deep4`; it is recorded as NOT EXECUTED for this
tree rather than reattributed.

2. **`%FF` rewrite literals (`module-api`).** The rewrite engine now also runs
the pattern byte-wise (`regex::bytes`, Unicode disabled) over the decoded path
when that path is not UTF-8, so `^/i.ndex$` matches `/i\xFFndex` and the capture
is re-encoded to `%FF` on the way out, as nginx does; the text rule on the URI
form still runs first, so existing rules keep their behaviour. New
`escape_uri_bytes` in `module-api/src/path_canon.rs`, `finish_rule` factored out,
four new/renamed unit tests (11/11 in the module) and the negative control
(`re_bytes = None`) failing the new test. ADR-048 § *Matching form* states the
resulting order.

3. **`when.path` on a non-UTF-8 target.** The condition that guards a rule now
follows the same order as the pattern: the URI form first, then the decoded
bytes through its own byte-wise twin, so `when.path` no longer vetoes a rule
that matches the same bytes. `CompiledWhen::Path` carries `{ re, re_bytes }`,
`when_match` takes the optional byte haystack, two unit tests cover the positive
and the veto cases (13/13 in the module) and the negative control (`|| false &&
byte_haystack…`) fails the positive one. ADR-048 § *Matching form* no longer
declares a `when.path` boundary.

**Tooling increment (same tree, not product code).** `scripts/security-fuzz.sh`
now has a documented `--mode main` matrix: the eight `[[bin]]` targets from
`fuzz/Cargo.toml`, a per-target `-max_total_time` budget, one log and one
`meta.env` per target, `summary.main.txt`, and a fail-closed exit when
`cargo-fuzz` or the nightly toolchain is missing (it used to print “skipping”
and exit 0). `.github/workflows/security-nightly.yml` gained a weekly
`fuzz-main` job that runs that matrix and uploads the evidence. Locally the
matrix ran 8/8 PASS with `FAIL=0`, exit 0, 480 s of fuzzing and zero artifacts;
the hosted run of the new job is **NOT EXECUTED** (it needs a push) and the
workflow was validated syntactically (YAML parse), not by `actionlint`. The same
increment registered NG-21's Linux cross check as `scripts/gates/linux-cross-check.sh`
plus the `linux-cross-check` job in `ci.yml` (locally `LINUX_CHECK_EXIT=0` and
`LINUX_ARM_CHECK_EXIT=0`, `LINUX_CROSS_CHECK=PASS`, negative control failing
when the sysroot is absent; its four Ubuntu `.deb` downloads are now pinned by
SHA-256 taken from the noble-updates/main `Packages` index — a mismatch discards
the file and re-fetches once, a second mismatch aborts before unpacking, and a
wrong pin was measured failing closed with RC 1 — and that work corrected the
arm64 pool host `/ubuntu` → `/ubuntu-ports`, which 404s on a cold sysroot that
the earlier green run had masked by reusing cached `.deb`s), and fixed `--ci` so it writes its generated corpus
under `target/` instead of next to the tracked seeds in `fuzz/corpus/`.

A later review of that same tooling left its behaviour unchanged and the code
smaller: one `PINNED_PACKAGES` table replaces two version constants plus four
version-bearing hash keys; `fetch_deb()` is the single verify-or-download path
(a failed download no longer leaves a partial file behind); `unpack_deb()` uses
one `tar -xf` instead of three compression branches; the redundant
`rustc --print target-list` check and an unreachable exit-label arm are gone;
arguments that take a value now fail closed (`--target` with no value used to
print `LINUX_CROSS_CHECK=PASS` without checking anything — measured RC 2); the
sysroot records its pins in `.pinned-debs` so a version bump cannot silently
reuse bytes from the previous `.debs`; and the job's explicit `rust-cache` key
was dropped because the action already scopes by job. Two defects were found by
running it, not by reading it: an uppercased cc-rs variable name dropped
`--sysroot` from the C build (`fatal error: 'stdlib.h' file not found`, five
RC 1 runs) and a stale-pin `--no-download` path deleted the sysroot before
refusing to rebuild it. After the fixes: macOS A–K (syntax, help, two bad-usage
cases, fresh sysroot, cache reuse, tampered `.deb`, stale stamp, two
`--no-download` cases, legacy sysroots) and a Linux arm64 container pass with
both targets plus the tamper negative, all tabulated in the roadmap with their
RCs. The hosted job is still **NOT EXECUTED** (it needs a push).

**Deep leg: the severity gate (owner decision).** The leg's six tools all pass
and `cargo clippy` itself exits **0**; what returns RC 1 is the auditor's
severity gate, fed by diagnostics from lints it forces on its own command line:
`-W clippy::unwrap_used, clippy::expect_used, clippy::panic,
clippy::indexing_slicing, clippy::todo, clippy::unimplemented,
clippy::arithmetic_side_effects, clippy::await_holding_lock`. These are
`pedantic`/`restriction` lints, allow-by-default and opt-in in clippy's own
design; the repository enables none of them (no `clippy.toml`, no
`[workspace.lints]`, no crate-level `warn`), which is why the project gate
`cargo clippy --workspace --all-targets --locked -- -D warnings` is green with
RC **0** (16.3 s, re-run in this increment). Full inventory, one row per
diagnostic (`severity`, `lint`, `file`, `line`):
`.exyonq-local/logs/clippy-deep3-inventory.tsv` (identical counts in the
`report/deep4` run) — 9035 rows = 2586
`expect_used` + 2270 `unwrap_used` + 2204 `indexing_slicing` + 1810
`arithmetic_side_effects` + 165 `panic`. The 5021 `high` split as **547 in
production code, 1971 inside inline `#[cfg(test)]` modules and 2503 in test
files** (343 files). Production top: `core/src/server/handler.rs` 103,
`core/src/server/wire_dispatch.rs` 47, `crates/exyonq-cache/src/store.rs` 32,
`core/src/server/mod.rs` 27, `config/ir/src/canonical.rs` 24 (per-file list in
`.exyonq-local/logs/clippy-high-production.tsv`, per-crate in
`.exyonq-local/logs/clippy-high-src-by-crate.tsv`: `crates/` 1261, `core/` 509,
`config/` 351 before the inline-test split). Closing them is not a mechanical
fix: every `unwrap`/`expect`/`panic` in production is a deliberate
panic-on-invariant decision, so replacing them changes observable behaviour
(panic → error paths), and the ~2503 in tests are assertions that must keep
panicking. Both are outside what this seal may change unilaterally, and
suppressing the lints (`#[allow]`, `rust-audit.toml` rule disabling, or a
narrower `--fail-on`) would hide real findings instead of repairing them, so
none was added: **0 diagnostics suppressed**. The leg is therefore reported as
NOT green at gate level, with the three tools' verdicts intact. Owner options:
(a) align the auditor's clippy step with the project lint policy (one tool-side
change, every other rule preserved); (b) authorize a per-crate hardening
campaign that converts panic-on-invariant sites into typed errors and keeps the
tests' assertions; (c) keep RC 1 documented as the accepted state. Until one is
chosen, `security --deep` is not a pass.

**Blocked by the environment, with the exact command and error.** `cargo vet
check` first failed with `exit 255` — `You must run 'cargo vet init' (store not
found under the repository's `supply-chain/`)`; that was resolved in this seal by creating the
store (*Vet store* above), so what remains is the owner's imports policy, not a
blocker. `cargo unmaintained` and `cargo supply-chain` are not installed
(`exit 101`, `no such command`); the same ground is covered by `cargo audit`,
`cargo deny`, `guarddog` and `osv-scanner`, which all ran. Miri cannot execute
tests that spawn processes under its default isolation (exact error above), and a
workspace-wide `cargo mutants` run exceeds any per-tool budget on this machine
(SIGKILL at 1800 s); the scoped run is the mutation evidence.

## Wire and route parity

0.9.3 changes **no** routing, auth, static-serving, request-path, TLS, WAF or
proxy-forwarding code, so the mandatory cross-release parity matrix is carried
by 0.9.2. The patch was still exercised end to end with the real release binary:

| Case | Request | Expected | Observed | PoC |
| --- | --- | --- | --- | --- |
| Proxied GET, real origin | `GET /api/health` | `200`, origin echoes path/method | `200`, `{"path":"/api/health","method":"GET",…}` | `.exyonq-local/smoke-0.9.3/proxy-smoke.sh` (log `.exyonq-local/logs/proxy-smoke-0.9.3-20261120.log`) |
| Query preservation | `GET /api/item?a=1&b=two` | origin sees `a=1&b=two` | `{"query":"a=1&b=two"}` | same |
| Body forwarding | `POST /api/upload`, 64 KiB text | origin sees 65 536 B, sha256 match | `body_len=65536`, sha256 `2ca84d87…` | same |
| Unmatched path | `GET /outside/route` | `404`, origin untouched | `404`, upstream log shows only the 3 routed requests | same |
| Default WAF (informational) | `GET /api/item?id=1' OR '1'='1` | blocked | `403` | same |

Result `proxy-smoke: PASS=5 FAIL=0` (DEVELOPMENT_AID — local upstream,
not E2E). The authoritative Linux suite
`scripts/e2e/proxy-e2e.sh` is **NOT EXECUTED** here: `RESULT=NOT_APPLICABLE
LINUX_REQUIRED platform=Darwin LOCAL_ITERATION_ONLY` (exit 2). Native Linux
amd64/arm64 parity against the 0.9.3 binary remains pending for the tag.

## Blockers

None

## Findings (open)

None.

## Findings closed in this seal

**F-01 (low, test strength) — closed.** The scoped mutation run over
`exyonq-addon-api` had left 12 survivors in `addon-api/src/lib.rs`
(`matches_host`→`true`, `Display`→`Ok(default)`, `allows_cost_class` with `!`
deleted, `register` `==`→`!=`, `modules`→empty leak, `len`→`0`/`1`,
`is_empty`→`true`/`false`, `pre` field deleted, `ensure_capabilities`→`Ok(())`
and its `!` deleted). Seven tests were added to the crate's existing test module,
pinning the contract each mutant breaks — no `#[mutants::skip]`, no
`#[cfg(not(mutants))]`, no new `--exclude`/`--skip`, and no change to the public
API, so every survivor must now die on a real assertion:

| Test (in `addon-api/src/lib.rs`) | Contract pinned | Mutants it kills |
| --- | --- | --- |
| `api_version_matches_only_the_same_major_and_displays_both_components` | major-only host match; `"major.minor"` rendering | :36 `matches_host`→`true`; :42 `Display`→`Ok(default)` |
| `cost_class_matrix_is_exact_per_profile` | Edge admits only the two cheap classes; Lb denies `Experimental`; Dev admits all | :79 deleted `!` in `Lb` |
| `registry_counts_and_modules_track_registrations` | `len`/`is_empty`/`modules` reflect the real `Vec` | :290, :294 (both), :298 (both) |
| `duplicate_names_are_rejected_and_unique_names_register` | duplicate name rejected by equality, not accepted, and not pushed | :282 `register` `==`→`!=` |
| `handshake_rejects_api_major_mismatch` | API major mismatch is a handshake error | :36 (also exercised end-to-end) |
| `prerelease_core_is_checked_as_its_release_line` | a core prerelease is compared as its release line, newer cores still rejected | :324 deleted `pre` field (prerelease preserved) |
| `capabilities_must_be_declared_by_some_addon` | missing capability errors, declared capability passes | :355 `Ok(())`; :360 deleted `!` |

A pre-existing suite race surfaced while re-running the gates and is fixed in
the same increment: `dns_refresh::tests` share the process-global `DNS_STATS`
table and its epoch, and a concurrent `replace_generation` prunes every peer of
another generation, which deleted a row between a test's `refresh_peer` write
and its assertion (`no entry found for key`, `crates/exyonq-mod-proxy/src/dns_refresh.rs`).
The fix is a test-only `tokio::sync::Mutex` that serialises the five tests
touching that global state: no product behaviour changes, no test assertion is
relaxed, and the crate's 168 tests now pass on five consecutive runs.

Verdict after the change: `31 mutants tested in 13m: 23 caught, 8 unviable,
0 missed, 0 timeout`, `cargo mutants` **exit 0** (the 8 unviable are mutants that
do not compile, unchanged from the baseline). Reproduction:
`cargo mutants --no-shuffle -p exyonq-addon-sdk -p exyonq-addon-api --timeout 30 --jobs 8`
(log `.exyonq-local/logs/mutants-f01-20261120.log`, evidence
`report/deep3/`); scope remains the two addon crates.

## Reviewed and dismissed

Rust Audit reports **33** criticals and the sealed report expands all 33 by site
(`report/rust-audit-20261120-033414.md`: R01 × 1, R12 × 27, R18d × 5; 33 section
headers, and the JSON re-run of the same scan on the same clean export agrees —
`report/clean093-recheck/rust-audit-20261120-222933.json`, summary
`critical: 33`, `by_rule` R18d 5 / R12 27 / R01 1). Every one is adjudicated
here: the 28 R01/R12 findings in this first table, one row per distinct site
(the `×N` counts are individual findings and add up to 28: R01 × 1 + R12 × 27),
and the 5 R18d names in the second table (33 = 28 + 5).
Every critical was read in the source; each R18d was checked against
`Cargo.lock`.

| Rule | Site | Why it is not a defect |
| --- | --- | --- |
| R01 (stub) | `scripts/gates/fixtures/must-fail/path-cache/bad.rs:6` | Deliberate must-fail fixture: it exists so the integrity gate can prove it detects `todo!()`. Fixtures are not product code. |
| R12 (secret-like literal) ×6 | `crates/exyonq-observability/src/redaction.rs:85-89,91` | Redaction **needles** (`cookie=`, `password=`, `bearer `…), the opposite of a secret. (Line 90 holds no needle, which is why the report has no finding there.) |
| R12 ×1 | `config/ir/src/diagnostic.rs:528` | Same class: diagnostic redaction needle list. |
| R12 ×2 | `config/ir/src/lib.rs:3512,3518` | TLS cipher-suite **names** (`TLS_ECDHE_…`, `ECDHE-…`) in a validity table. |
| R12 ×2 | `crates/exyonq-mod-tls/src/lib.rs:493,502` | Same cipher-suite name mapping. |
| R12 ×5 | `crates/exyonq-observability/src/bounds.rs:57,61,73,87,99` | Bound descriptions (`drop_newest_when_full`, `sdk_batch_drop_when_full`…). |
| R12 ×1 | `crates/exyonq-observability/src/install.rs:625` | Reload-policy identifiers (`RELOADABLE_SIZE_BASED`, `HTTP_PROTOBUF_ONLY_GRPC_REJECT`). |
| R12 ×2 | `crates/exyonq-cfd-dataplane/src/obs/hub.rs:466,557` | Prometheus metric names. |
| R12 ×1 | `core/src/server/handler.rs:67` | Response header name `x-exyonq-materialization-budget-exceeded`. |
| R12 ×1 | `module-api/src/static_dispatch.rs:306` | Same header name as a public constant. |
| R12 ×5 | `benchmarks/tools/http-loadgen/src/lib.rs:57-62` | SHA-256 checksums of the benchmark payload oracles. |
| R12 ×1 | `benchmarks/tools/http3-loadgen/src/lib.rs:44` | Same payload checksum. |

### R18d heuristics (5 sites): the tool's offline tables, not the code

R18d is the "impossible version / hallucinated crate name" heuristic. Offline it
cannot query crates.io, so it judges from a baked-in major-version table and a
name-shape list; the four `Cargo.toml`/loadgen rows below are resolved
successfully from crates.io by this very lockfile, and the fifth names a
workspace crate, not a registry one.

| Rule | Site | Why it is not a defect |
| --- | --- | --- |
| R18d (impossible version) | `Cargo.toml:1` — `thiserror = "2"` | Report says "published majors end at 1"; `Cargo.lock` resolves **`thiserror 2.0.21`** from crates.io and the workspace builds with it. The tool's offline major table is stale, not the requirement. |
| R18d (impossible version) | `Cargo.toml:1` — `dashmap = "6"` | Same class: `Cargo.lock` resolves **`dashmap 6.2.1`**; the "ends at 5" table row is stale. |
| R18d (naming shape) | `benchmarks/tools/http-loadgen/src/lib.rs:25` — `http_body_util` | Real crate: package `http-body-util 0.1.5` (hyphenated on crates.io, underscored in `use`) is in `Cargo.lock` and used by the load generator. |
| R18d (naming shape) | `benchmarks/tools/http-loadgen/src/lib.rs:35` — `hyper_util` | Real crate: package `hyper-util 0.1.21` in `Cargo.lock`. |
| R18d (naming shape) | `cli/exyonq/src/l2_coord_lab.rs:23` — `exyonq_core` | Workspace crate `exyonq-core` (`core/Cargo.toml:2`), a path dependency of `cli/exyonq`; the underscore is the Rust import name, not a registry slug. |

High-severity classes were reviewed by rule, not one by one, matching the 0.9.2
practice: R09 panic/`expect` in production code (235), R03 functions that return
a constant or ignore parameters (86 — trait impls, platform stubs and
`#[cfg(test)]` helpers), R11 `unsafe`/`*const` without a recognised SAFETY
comment (53 — `core/src/server/io.rs` and `crates/exyonq-linux-ffi/*`, where the
ADR-045 ratchet `unsafe-allowlist-gate.sh --tree .` PASSes), R20 typosquat
heuristic (11 — e.g. `http` vs `tap`), R16 sync lock across `.await` (11 — the
flagged `fcgi_metrics_assert_guard`/`proxy_metrics_assert_guard` use
`tokio::sync::Mutex`, are `#[doc(hidden)]` test helpers, and holding that guard
across `.await` is their purpose), R21 `build.rs` external command (2 —
`cli/exyonq/build.rs` and `cli/exyonqctl/build.rs` run `git describe
--tags --exact-match` to embed `source_tag`, which is the EXY-14 fix itself).

## Current-tree findings review (2026-11-25)

Rust Audit 0.2.9 reports **28 criticals** on the current tree, all from two
rules, and the report was read at every critical site:

| Rule | Count | Sites in `report/rust-audit-20261125-225601.md` | Why it is not a defect |
| --- | --- | --- | --- |
| R12 (secret-like literal) | 27 | `crates/exyonq-observability/src/redaction.rs:85-91` (6), `config/ir/src/diagnostic.rs:528` (1), `config/ir/src/lib.rs:3514,3520` (2), `crates/exyonq-mod-tls/src/lib.rs:493,502` (2), `crates/exyonq-observability/src/bounds.rs:57-99` (5), `crates/exyonq-observability/src/install.rs:625` (1), `crates/exyonq-cfd-dataplane/src/obs/hub.rs:466,557` (2), `core/src/server/handler.rs:67` (1), `module-api/src/static_dispatch.rs:306` (1), `benchmarks/tools/http-loadgen/src/lib.rs:57-62` (5), `benchmarks/tools/http3-loadgen/src/lib.rs:44` (1) | Redaction needles, TLS cipher-suite names, response-header/metric identifiers and benchmark payload checksums — the same classifier false positives adjudicated in the historical table below. |
| R18d (offline dependency table) | 1 | `Cargo.toml:1` | The tool's baked-in major table is stale for `thiserror = "2"` / `dashmap = "6"`; `Cargo.lock` resolves 2.0.21 / 6.2.1 and the workspace builds. |

The **118 highs** were reviewed by rule, not one by one, matching the 0.9.2
practice: R09 panic/`expect`, R03 constant/ignored-parameter heuristics, R11
`unsafe` without a recognised SAFETY comment (the ADR-045 ratchet
`unsafe-allowlist-gate.sh --tree .` PASSes), R20 typosquat names, R16 sync lock
across `.await` in test helpers, R21 `build.rs` `git describe` calls, and R18a
unresolved `use` paths in test modules (the tests compile and run). The FastCGI
and ACME/H3 increments add no critical and no new real high; the one R18a in
`crates/exyonq-mod-fastcgi/tests/unix_transport_integration.rs:18` is the
module-import heuristic, and the quiche R09 sites predate this increment. No
finding is waived: `--fail-on high` remains RC 1 and is reported as such.

## Findings (closed this release)

| ID | Severity | Resolution |
| --- | --- | --- |
| T-01 | medium (test isolation) | `exyonq-metrics` `wire_terminal_statuses_are_accounted_once_by_real_class` failed in a workspace run with `left: 0, right: 8`: two tests in the same binary toggle the process-global `wire_enabled` switch, so one reset made the other's `wire_record_*` calls no-ops. Fixed in `modules/metrics/src/lib.rs` with a test-only `WIRE_SWITCH_LOCK` guard held by both toggling tests. Re-verified: `cargo test -p exyonq-metrics --lib` 20 consecutive runs, 0 failures; full CI green. |
| T-02 | medium (test isolation) | `exyonq-config-cli` `provenance_root_scalar_line` failed with `read …/frag.toml: No such file or directory`: `tmp()` named the directory with nanoseconds only, so two tests could share it and overwrite `main.toml`/`frag.toml`. Fixed in `config/cli/src/provenance.rs` (pid + atomic counter, `fs::create_dir` so a collision fails loudly). Re-verified: `cargo test -p exyonq-config-cli --lib` 25 consecutive runs, 0 failures. |
| T-03 | medium (dependency, unsound) | `RUSTSEC-2026-0486` (`rustix` `xattr` marks unwritten bytes as initialized for a zero-length buffer) hit `rustix 1.1.5`, reachable from the workspace lock through the `tempfile` dev-dependency. The advisory is fixed in 1.1.6 and the patch was applied: `cargo update -p rustix --precise 1.1.6` (lock-only change; the release binary sha256 did not move). Re-verified: `cargo audit` RC 0 with **no warning**, `cargo deny check advisories bans sources` → `advisories ok, bans ok, sources ok`, `osv-scanner scan source --lockfile=Cargo.lock` → **0 findings**, full CI green (**2322 passed**). |
| T-04 | low (test gap, contract) | Mutation testing found the addon compatibility contract unasserted: `cargo mutants -p exyonq-addon-sdk` reported **2 missed / 1 unviable** at `addon-sdk/src/lib.rs:46` (`official_core_compat` returning `">=0.1.0, <0.10.0"`); replacing the string with `""` or `"xyzzy"` broke no test. Fixed with `official_core_compat_pins_the_0_9_line`, which parses the requirement and asserts it admits 0.9.x and 0.1.0 while rejecting 0.10.0 and 1.0.0. Re-verified: the same scoped run now reports **2 caught / 1 unviable, 0 missed, RC 0**, and CI is green. |
| T-05 | medium (dependency, reachable at runtime) | `RUSTSEC-2026-0592` (`Url::origin()` stack overflow on nested `blob:` URLs, published 2026-11-20 — after this seal was written) hit `url 2.5.8`, reachable from `redis 1.8.2`, `reqwest 0.13.5` (`exyonq-control-api-client`, `exyonq-observability`) and `tower-http 0.6.11`. Fixed in 2.5.9 and applied as a lock-only patch bump: `cargo update -p url --precise 2.5.9` (`url 2.5.8 -> 2.5.9`, `form_urlencoded 1.2.2 -> 1.2.3`, its own direct dependency); no manifest, source or behaviour change. Re-verified 2026-11-23: `cargo audit` **RC 0, 0 findings** (519 dependencies), `cargo check --workspace --locked` **RC 0** (22.15 s), `cargo test -p exyonq-cache-redis --locked` **39 passed / 0 failed**, `cargo test -p exyonq-observability --locked` **10 passed / 0 failed**, `cargo test -p exyonq-control-api-client --locked` **RC 0** (crate has no tests of its own). The Veyra and Rust Audit rows above predate this lock change and were **not** re-run for it. |

No product behaviour changed for these fixes: T-01/T-02 were test scaffolding, T-03
is a patch-level lock bump of a dev-only transitive dependency, and T-04 pins an
existing contract in a test. All five were found by running the tools, not by
weakening a gate. T-05 is the only entry that post-dates the audit legs above: it
was surfaced by `cargo audit` on 2026-11-23 and fixed in the lock, so the legs
1–2 rows describe the tree before it.

## Waivers

| ID | Severity | Reason | Mitigation | Expires |
| --- | --- | --- | --- | --- |
| — | — | — | — | — |

## Surfaces reviewed

- [x] Dependency pin `wasmtime = "=49.0.6"` and the advisory set it clears
- [x] Workspace version and release-tooling version allowlist (0.9.3 accepted, legacy still rejected)
- [x] EXY-14 `source_tag`/`source_revision` gate, now wired into `xtask ci` and `release.yml`
- [x] Access-event `host` and WAF violation fields (#84); packaged healthcheck unit discovery (#81)
- [x] Release-binary proxied path: GET, query, body integrity, 404 isolation, default WAF
- [x] Test isolation of process-global state (`wire_enabled`, provenance temp dirs)
- [x] Supply-chain: `cargo audit`, `cargo deny`, `guarddog`, `osv-scanner`, and a real `cargo vet` store (`supply-chain/`, 464 baseline exemptions)
- [x] Mutation testing of the addon contract (`exyonq-addon-sdk`, `exyonq-addon-api`), scope stated
- [x] Private-material and publication-privacy surfaces
- [x] Issue #109 FastCGI/PHP-FPM fix: `errno` preflight, `ConnectFailure`, per-pool metrics, `exyonqctl fastcgi check`, response-body limit, docs and tests (2026-11-25)
- [x] ACME per `[[server]]`, H3 SNI multi-cert (s2n), local QUIC SNI check on Darwin arm64; the Linux-native TLS and `curl-http3` gates remain NOT EXECUTED
- [ ] Native Linux amd64/arm64 parity, images, Scout, signatures and GitHub Actions (NOT EXECUTED before the seal)

## Publication limitations

- No tag, commit, push, image or signature exists for 0.9.3 at seal time; those
  gates are executed by the tag workflow and recorded in
  `docs/release/v0.9.3.md` afterwards. Re-checked 2026-11-25: the verifier
  returns `CONTROL=worktree_fingerprint_mismatch` / RC 1 on a modified tracked or
  untracked tree and PASS / RC 0 only with this seal; a deliberately modified tracked file was restored
  byte-for-byte. Re-checked 2026-11-21 with the fail-closed
  gates: `tag-signature-gate.sh --tag v0.9.3 --expected-commit 0f58cae…` →
  `ERROR: tag must be annotated (got type='missing')`, `RESULT=FAIL`, RC 1;
  `SOURCE_TAG_EXPECT=v0.9.3` on the release binary → `FAIL empty source_tag`, RC 1
  (the bare gate stays `NOT_APPLICABLE`, never a pass);
  `generate-release-manifest.sh --version 0.9.3` stops at
  `ERROR: artifacts-dir is empty`; `cosign` is not installed and `gh secret list`
  has no rows. Log `.exyonq-local/logs/block-roadmap-170-20261121.log`.
- Veyra's `cargo-geiger` leg is INCONCLUSIVE (coverage warnings in 54 runs;
  unsafe counts are a lower bound) and `dylint`, `cargo-semver-checks` and
  `veyra-e2e` are SKIPPED (no lint config, no published baseline, no
  `[[e2e.scenarios]]`); none is reported as a pass. The Veyra `ACCEPTED`
  decision covers the required profile legs only.
- Rust Audit **0.2.9** was run on the current 84-entry tree with the exclusions
  stated in *Current-tree re-audit* (local copies, build output, must-fail
  fixtures). The **0.1.0** run and its four extra legs (`--supply-chain`,
  `--miri`, `--deep`, `scan --online`) belong to the historical 2026-11-20
  snapshot; they were **NOT EXECUTED** on this tree and are retained only as
  history. `cargo vet`/`cargo machete` historical results are likewise not
  reattributed. Still NOT EXECUTED on the current tree: `cargo udeps` and
  `cargo unmaintained` (not installed), a workspace-wide mutation run (exceeds
  the per-tool budget), and the four extra Rust Audit legs above.
- CodeMap is advisory: the current index reports **6 gaps** (not the truncated
  400 of the earlier run) and a missing edge is not proof of absence.
- The proxied smoke ran on macOS arm64 against a local Python origin; it is not
  a substitute for the Linux-only authoritative suite.

## Final verification (2026-11-25)

| Command | Result |
| --- | --- |
| `bash scripts/verify-release-audit.sh --selftest` | `RELEASE_AUDIT_SELFTEST=PASS`, `RELEASE_AUDIT_MISSING_PATHS=0`, RC **0**; covers `clean_tree_unverifiable`, `worktree_diff_mismatch`, `worktree_diff_match`, `legacy_no_fingerprint`, `audit_seal_clean_unverifiable`, `untracked_content_match`, `untracked_content_mismatch`, `untracked_content_restored`, `untracked_add_delete_base`, `untracked_added`, `untracked_add_removed`, `untracked_deleted`, `untracked_delete_restored` and `citation_mismatch` |
| `bash scripts/verify-release-audit.sh 0.9.3` | `WORKTREE_FINGERPRINT=MATCH fingerprint=89788d3c… files=84`, `RELEASE_AUDIT_GATE=PASS`, `Release audit OK: audit-v0.9.3.md @ 0f58cae…`, RC **0** |
| Mismatch controls (temporary tracked append and a separate untracked content edit, each restored byte-exactly) | `CONTROL=worktree_fingerprint_mismatch` with both sealed and current fingerprints / **84** entries, RC **1**; after exact restore the verifier returned PASS / RC 0 with the original fingerprint. The untracked case reported the changed path. |
| `bash scripts/gates/q13-docs-audit.sh` | `Q13_FRAGMENTS=29/31 skipped=2 allowlisted=0 failed=0`, `RESULT=PASS Q13_DOCS_AUDIT`, RC **0** |
| `bash scripts/gates/real-evidence-gate.sh --files docs/release/v0.9.3.md docs/security/audit-v0.9.3.md` | `REAL_EVIDENCE_GATE=PASS`, RC **0** |
| `git diff --check` | RC **0** (no whitespace errors) |

**No bypass paths.** A clean tree has an empty `git diff`, so the sealed
tracked component and untracked manifest cannot be checked together; the verifier now fails explicitly with
`CONTROL=worktree_fingerprint_unverifiable` instead of printing
`NOT_APPLICABLE`. A legacy seal without the tracked, untracked and combined
fingerprint fields fails with `CONTROL=worktree_fingerprint_missing`. All of
those are selftest cases (`PASS_EXPECT_FAIL clean_tree_unverifiable`,
`PASS_EXPECT_FAIL legacy_no_fingerprint`, `PASS_EXPECT_FAIL untracked_content_mismatch`,
`PASS_EXPECT_FAIL untracked_added`, `PASS_EXPECT_FAIL untracked_deleted` and
`PASS_EXPECT_FAIL citation_mismatch`), as is the matching dirty tree
(`PASS_EXPECT_PASS worktree_diff_match`). Tag-time verification must therefore
be run on the audited dirty snapshot; a clean checkout is not accepted as proof
of this seal.

## Sign-off

- **Reviewer:** local release pipeline (commands, exit codes and paths above; no external reviewer)
- **Date:** 2026-11-25
- **Notes:** Local audit seal for the 84-entry 0.9.3 patch, now committed as
  `d145976` with this document as the audit-only seal commit on top. Veyra
  standard, Rust Audit 0.2.9, CodeMap and the rebuilt release binary were run on
  this tree; missing post-publication or host-bound gates are reported as NOT
  EXECUTED here and belong in `docs/release/v0.9.3.md`, never as passes.
