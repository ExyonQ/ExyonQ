# P14V041-SCOPE — ExyonQ v0.4.1 Scope and Dependency Audit

```text
DOCUMENT = docs/releases/p14v041-scope-audit.md
AUDIT_ID = P14V041-SCOPE
AUDIT_DATE = 2026-07-28
AUDIT_KIND = SCOPE_PROPOSAL_ONLY
IMPLEMENTATION_PHASE_OPENED = NO
PUBLICATION_STATUS = FORBIDDEN
PUSH = NO
TAG = NO
RELEASE = NO
LATEST_CHANGED = NO
VISIBILITY_CHANGED = NO
```

Read-only inspection of both trees. No dependency changes, no product commits, no sync between trees.

---

## Status block (required)

```text
P14V041_AUDIT_STATUS = COMPLETE_PROPOSAL
P14V041_BASE_HEAD_DEVELOPMENT = 9dad9587c94f3732c1c3f8bc8153ac3b26f6e0bc
P14V041_BASE_HEAD_GITHUB = d326b02b4ebc3b8dd7a8a3dbe7deeab5251d4910
P14V041_TREES_RECONCILED = NO
P14V041_UNCOMMITTED_CHANGES = YES
P14V041_CURRENT_MSRV = UNDECLARED_IN_CARGO_TOML
P14V041_CURRENT_RUST_TOOLCHAIN = 1.93.0

P14V041_SCOPE = PRIVATE_MATURATION_PATCH_RELEASE
P14V041_IMPLEMENTATION_READY = NO
P14V041_IMPLEMENTATION_PHASE_OPENED = NO

PUBLICATION_STATUS = FORBIDDEN
PUSH = NO
TAG = NO
RELEASE = NO
LATEST_CHANGED = NO
VISIBILITY_CHANGED = NO
```

Canonical published `v0.4.0` (authoritative for this audit):

```text
CURRENT_VERSION = 0.4.0
P14V040_CANONICAL_HEAD = d326b02b4ebc3b8dd7a8a3dbe7deeab5251d4910
P14V040_STATUS = PRIVATE_V040_RELEASE_COMPLETE
P14HISTCLEAN_STATUS = COMPLETE
P14HISTCLEAN_REMOTE_FULL_HISTORY_SCAN = PASS
EXYONQ_SEC_PRIVATE_MATERIAL_ZERO_GATE = PASS (as closed for v0.4.0)
GITHUB_REPOSITORY_VISIBILITY = PRIVATE
GHCR_VISIBILITY = PRIVATE
LATEST_TAG_AUTHORIZED = NO
PUBLIC_OPENING_AUTHORIZED = NO
```

---

## 1. Tree inspection

### 1.1 Development (`/Volumes/Lexar/Cursor/exyonq-lab`)

| Field | Value |
|-------|-------|
| Branch | `perf/p8o-finite-sse-batch` |
| HEAD | `9dad9587c94f3732c1c3f8bc8153ac3b26f6e0bc` |
| `main` | `1c43f718f91c4fd115b70b919439644af4d64ac2` |
| Workspace version on current HEAD | `0.3.3` (not 0.4.0) |
| `v0.4.0` peel | `d326b02b4ebc3b8dd7a8a3dbe7deeab5251d4910` (tag present) |
| Dirty paths | ~150 (`53` modified, `97` untracked) |
| Dirty top areas | `docs/`, `scripts/`, `core/`, `benchmarks/`, plus `Cargo.toml`/`Cargo.lock`, workflows, addon/module surfaces |

`d326b02` is **not** an ancestor of current lab `HEAD` or lab `main` (`merge-base --is-ancestor` fails). Histclean rewrote the published GitHub line; lab continues a divergent local history plus ambient dirt.

Local `.exyonq-local/status/p14v040-STATUS.env` still shows stale `OPEN` / Phase-2 blocked state. Authoritative close evidence is under `.exyonq-local/release/p14v040/phase5-histclean-…/evidence/phase6-final-status.env` (`PUBLICATION_STATUS=PRIVATE_V040_RELEASE_COMPLETE`). Status-file refresh is **out of scope** for this audit.

### 1.2 GitHub canonical (`/Volumes/Lexar/Cursor/exyonq-github`)

| Field | Value |
|-------|-------|
| Branch | `main` |
| HEAD | `d326b02b4ebc3b8dd7a8a3dbe7deeab5251d4910` |
| Working tree | clean |
| Sync with `origin/main` | up to date (inspection only; no push) |
| Workspace version | `0.4.0` |
| `v0.4.0` peel | `d326b02…` |
| Toolchain file | `rust-toolchain.toml` → `channel = "1.93.0"` |

### 1.3 Reconciliation verdict

```text
P14V041_TREES_RECONCILED = NO
```

- GitHub tree matches the published freeze.
- Development tree is on an unrelated perf branch with large unclassified dirt and a divergent `main`.
- **v0.4.1 work must start from a clean lab checkout of `d326b02` / `v0.4.0`**, without mixing ambient `perf/*`, bench results, or pre-histclean history into the patch release.

No files were copied between trees during this audit.

---

## 2. Toolchain / MSRV

| Item | Evidence |
|------|----------|
| Effective toolchain | `rust-toolchain.toml` = `1.93.0` (both trees) |
| Local `rustc` / `cargo` | `1.93.0` / `1.93.0` (audit host) |
| `package.rust-version` / workspace MSRV field | **absent** in `Cargo.toml` |
| README | “see `rust-toolchain.toml`” |

```text
P14V041_CURRENT_MSRV = UNDECLARED_IN_CARGO_TOML
P14V041_CURRENT_RUST_TOOLCHAIN = 1.93.0
```

Policy for v0.4.1: do **not** raise toolchain or introduce incidental MSRV elevation. Candidate crates checked against crates.io `rust_version` (below) remain below 1.93.0 except `notify 9.0.0-rc.*` (MSRV 1.85 **and** prerelease → excluded).

---

## 3. Workflows, Dependabot, Actions

### 3.1 Active workflows (GitHub `v0.4.0`)

- `.github/workflows/ci.yml`
- `.github/workflows/security.yml`
- `.github/workflows/security-nightly.yml`
- `.github/workflows/release.yml`
- `.github/workflows/deps-wasmtime-weekly.yml`

### 3.2 Dependabot

`.github/dependabot.yml`: weekly cargo + github-actions (Europe/Madrid), open-PR limit 5.

Deferred explicitly from `docs/release/p14v040-phase0-notes.md` to **0.4.1**:

| PR | Proposal |
|----|----------|
| #7 | bytes 1.12.0 → 1.12.1 |
| #6 | anyhow 1.0.103 → 1.0.104 |
| #5 | socket2 0.5.10 → 0.6.4 |
| #4 | serde 1.0.228 → 1.0.229 |
| #3 | notify 7.0.0 → 8.2.0 |
| #2 | actions/cache 5 → 6 |
| #1 | actions/setup-go 6 → 7 |

Remote Dependabot branches also exist for criterion, io-uring, matchit, memmap2, quinn, rustls, wasmtime, and other Actions — **not** admitted by default into v0.4.1.

### 3.3 `actions/cache` / `actions/setup-go`

| Action | Current use | Current pin style | Latest stable observed | Decision |
|--------|-------------|-------------------|------------------------|----------|
| `actions/cache` | **Active** — `ci.yml` (cargo-semver-checks tools), `security.yml` (cargo-deny/about tools) at `@v5` | Floating major tag (not immutable SHA) | `v6` / `v6.1.0` (tag `v6` → commit `55cc8345…`) | `UPDATE_V041` (isolated CI commit) + SHA pin |
| `actions/setup-go` | **Active** — `release.yml` `package-linux` installs pinned `nfpm` via Go | `@v6` floating tag (tag `v6` → `924ae3a1…`) | `v7.0.0` (tag `v7` → `b7ad1dad…`) | `UPDATE_V041` optional; **not** `REMOVE_UNUSED` |

`actions/setup-go` is required for the current nfpm packaging path. Removing it is not justified.

Cross-cutting CI finding (security hardening candidate): **no** `uses: …@<40-hex>` SHA pins observed; majors float (`@v5`/`@v6`/`@v7`/`@master` for `dtolnay/rust-toolchain`). v0.4.1 should pin by SHA with semantic version comments and must not widen workflow permissions or add secrets.

---

## 4. Advisories / security scripts (inspection)

On GitHub `d326b02` lockfile:

```text
cargo deny check advisories = PASS (advisories ok)
cargo audit = 0 vulnerabilities; 2 allowed unmaintained warnings
  - RUSTSEC-2024-0384 instant 0.1.13
  - RUSTSEC-2025-0134 rustls-pemfile 2.2.0 (already in deny.toml ignore)
```

Scripts present on GitHub freeze:

- `scripts/test-tls/generate-ephemeral-tls.sh`
- `scripts/test-tls/test-ephemeral-tls-and-scanner.sh`
- `scripts/test-tls/exyonq-sec-private-material-zero.sh`
- `scripts/security/scan-private-material.sh`

Policy to keep fail-closed:

```text
RULE_ID = EXYONQ-SEC-PRIVATE-MATERIAL-ZERO
FAIL_MODE = FAIL_CLOSED
```

No key backups or signing volumes were accessed.

---

## 5. Deferred findings from v0.4.0 (in scope awareness)

From `docs/release/p14v040-phase0-notes.md` and related release notes:

| Item | Disposition for v0.4.1 |
|------|-------------------------|
| Dependabot PRs #1–#7 | Primary dependency candidates (this audit) |
| OCI local crypto proof / OCI publication | **Do not reopen** as a publication phase; dual-arch OCI **smoke** only if later authorized under correctness/OCI criteria |
| Public opening / `latest` | Deferred to 0.4.5 / 0.5.0 — stay out |
| CHANGELOG stale “Signing deferred to P14SIGN” | Metadata honesty fix — eligible as docs-only in maturation |
| `rustls-pemfile` unmaintained | Track; migration → `0.4.2+` unless advisory severity rises |
| Dual `socket2` (DBEX-006) | Temporary; **unification not authorized** by Dependabot alone |
| Honesty limits (full htaccess, intergenerational FastCGI pool reuse, unsupported perf leadership) | Remain out of claims |

Roadmap / plans that define **post**-0.4.1 product work (defer): admin API / OpenAPI (`80`–`83`), htaccess hosting (`05B`/`11`), WordPress/cache enterprise, distributed discovery, kernel decomposition, major HTTP/3 rival tracks, WAF/Nexus-class surfaces.

---

## Scope proposal

```text
P14V041_SCOPE = PRIVATE_MATURATION_PATCH_RELEASE
```

Centered on: deferred dependency maintenance; demonstrable security hardening; bounded correctness/ops fixes found during updates; no-regression discipline; small performance deltas only with evidence on Netcup/Oracle.

Not a feature release.

---

## Priorities

```text
P14V041_PRIORITIES =

P0:
1. security and private-material zero
2. reproducible build from d326b02 base
3. tests and functional behavior
4. dependencies with advisory / incompatibility / required support

P1:
5. bytes 1.12.0 → 1.12.1
6. socket2 (see matrix — not Dependabot unify)
7. serde (+ serde_derive) 1.0.228 → 1.0.229
8. notify 7.0.0 → 8.2.0 (stable only)
9. anyhow 1.0.103 → 1.0.104

P2:
10. actions/cache v5 → v6 (SHA-pinned)
11. actions/setup-go v6 → v7 (SHA-pinned; keep for nfpm)
12. broader Actions SHA-pin pass + CI warning cleanup caused by updates

P3:
13. small performance wins only from measured hot-path deltas
14. non-blocking minor correctness found during the above
```

---

## Blockers (report only; not remediated)

```text
P14V041_BLOCKERS =

B1 = development tree not reconciled with published HEAD d326b02
     (lab HEAD/main diverge; histclean line not ancestor of lab main)

B2 = large unclassified local dirt (~150 paths) on development tree
     (must not enter v0.4.1 without classification / quarantine)

B3 = none demonstrated yet for proposed stable candidates vs toolchain 1.93.0
     (notify 9.0.0-rc.* would be B3 + prerelease forbid if proposed)

B4 = notify 7 → 8 is a major with EventType / MSRV 1.77 notes — treat as
     potential runtime/config-watcher behavior risk until tests pass

B5 = none measured yet (performance matrices pending implementation phase)

B6 = none Critical/High on frozen lockfile; unmaintained warnings tracked

B7 = none observed this audit; must re-run gates during implementation

B8 = any path that requires signing, publication, history rewrite, or
     visibility change remains forbidden — do not open

B9 = none required for this audit; implementation must not add unauthorized
     secrets/auth flows

B10 = actions/setup-go is used (nfpm) — do not classify as unused;
      Dependabot socket2 0.5→0.6 unify is policy-misaligned (DBEX-006)
```

Until **B1** and **B2** are resolved by maintainer process (clean base branch from `d326b02`, dirt quarantined), implementation must not start.

---

## Dependency updates (summary)

```text
P14V041_DEPENDENCY_UPDATES =
  INCLUDE: anyhow 1.0.104; bytes 1.12.1; serde/serde_derive 1.0.229;
           notify 8.2.0; actions/cache@v6; actions/setup-go@v7 (optional);
           Actions SHA-pin hardening
  CONDITIONAL: socket2 0.6.4 → 0.6.5 only on fastcgi side after changelog review;
               serde_json 1.0.150 → 1.0.151 only if pulled or clearly coupled
  EXCLUDE: notify 9.0.0-rc.*; socket2 platform-linux 0.5→0.6 unify;
           wasmtime 46.x; quinn/rustls mass bumps; criterion/io-uring/etc.
           unless a new advisory forces triage
```

---

## Security work

```text
P14V041_SECURITY_WORK =
  - Keep EXYONQ-SEC-PRIVATE-MATERIAL-ZERO fail-closed
  - Re-run ephemeral TLS + scanner scripts on worktree, tracked tree,
    build contexts, artifacts, OCI smoke inputs (no keybackup access)
  - cargo audit + cargo deny advisories on each dependency commit
  - SBOM / legal bundle delta review pre/post
  - Pin GitHub Actions by immutable SHA + semver comment
  - Review workflow permissions (no expansion; release.yml already
    scopes contents/packages per job)
  - No new generic private-material exceptions
  - Do not reopen histclean / signing / GHCR public / latest
```

---

## Correctness work

```text
P14V041_CORRECTNESS_WORK =
  Admit only:
  - reproducible defects found while updating deps
  - reload/config watcher issues if notify 8 changes behavior
  - startup/shutdown/drain failures
  - panic/deadlock/fd leak demonstrated
  - AMD64/ARM64 divergence
  - protocol regressions (static/proxy/FastCGI/TLS/H2/H3 smokes)
  - packaging/OCI smoke defects
  - security defects

  Exclude new broad features (WAF, Nexus, admin API, multinode,
  major H3 redesign, full htaccess, WP enterprise, kernel redesign,
  intergenerational FastCGI pool reuse).
```

---

## Performance work

```text
P14V041_PERFORMANCE_WORK =
  - No new competitive track
  - Required for bytes (hot path) and any socket2 change touching
    platform-linux / FastCGI connect path
  - Baseline = v0.4.0 / d326b02; same config, loadgen, limits, reps
  - Hosts = Netcup amd64 + Oracle arm64 when path affected
  - Metrics = RPS, latency, CPU, RSS, errors
  - PERFORMANCE_REGRESSION_INVESTIGATE = >3% median reproducible
  - Also investigate smaller but consistent / tail / error / arch-asymmetric deltas
  - Docker Desktop = LOCAL_ITERATION_ONLY / NOT_LINUX_EVIDENCE
```

---

## Release criteria (proposed)

```text
P14V041_RELEASE_CRITERIA =

SOURCE: version 0.4.1; clean tree; bisectable commits; no history rewrite;
        changelog complete

BUILD: cargo build workspace; --release; relevant feature matrix;
       system allocator; jemalloc when applicable

TEST: unit; integration; protocol smokes (static/proxy/FastCGI/TLS/H2/H3);
      config compatibility; CLI

SECURITY: private-material zero PASS; advisory review PASS; SBOM delta
          reviewed; no secrets; workflow permissions reviewed; Actions SHA-pinned

LINUX: Netcup amd64 PASS; Oracle arm64 PASS

PERFORMANCE: no material unexplained regression; raw evidence retained;
             hot-path dep deltas explicitly accepted

OCI: amd64/arm64/multi-arch smoke PASS when packaging path touched;
     no latest; no publication until later express authorize

RELEASE: signing manual outside Cursor; tag/release/GHCR need express
         authorize; repo + packages remain private
```

---

## Included vs deferred

```text
P14V041_INCLUDED_WORK =
  - Establish clean lab branch from d326b02 (process gate; pre-impl)
  - Isolated dependency commits per policy
  - Security gate retention + Actions hardening
  - Reload/watcher + smoke matrices
  - Hot-path regression control for bytes (/socket2 if changed)
  - Version bump 0.4.0 → 0.4.1 + changelog honesty fixes

P14V041_DEFERRED_TO_V042_PLUS =
  - WAF complete; Nexus Panel; public admin API; multinode
  - Major HTTP/3 redesign; global QUIC provider substitution
  - Full .htaccess compatibility; WordPress Enterprise
  - Kernel redesign; intergenerational FastCGI pool reuse
  - Speculative abstractions; unprofiled optimization
  - Public repository opening; latest tag
  - rustls-pemfile → rustls-pki-types migration
  - DBEX-006 socket2 dual-version elimination (policy FUTURE)
  - notify 9.0.0-rc.*
  - Wasmtime 46.x / unrelated Dependabot cargo PRs without advisory
  - Reopening P14V040 / P14HISTCLEAN / P14TLSFIX / signing / GHCR publication phases
```

---

## Dependency matrix

Versions verified via crates.io (2026-07-28) and GitHub `Cargo.lock` at `d326b02`. Target versions are from published stables + changelogs/release notes cited; not intuition.

### anyhow

| Field | Value |
|-------|-------|
| DEPENDENCY | anyhow |
| CURRENT_DIRECT_VERSION | `1` (workspace) |
| CURRENT_LOCKED_VERSION | `1.0.103` |
| TARGET_VERSION | `1.0.104` |
| DIRECT_OR_TRANSITIVE | direct (workspace; core, cli, xtask, modules, …) |
| USAGE_LOCATIONS | ~30 Rust crates; control/CLI/orchestration heavy; also some module paths |
| HOT_PATH | NO (prefer thiserror on stable façades; anyhow allowed in CLI/xtask) |
| SECURITY_RELEVANCE | LOW |
| BREAKING_CHANGES | None expected (patch; release notes: syn v3 as **dev**-dependency) |
| MSRV_DELTA | crates.io rust_version `1.68` — no raise vs 1.93.0 |
| FEATURE_DELTA | none expected |
| TRANSITIVE_DELTA | minor (syn for anyhow’s own tests only) |
| PLATFORM_DELTA | none |
| CONFIG_OR_API_DELTA | none |
| TEST_PLAN | `cargo check` workspace; `cargo test -p` affected; `cargo deny`/`audit` |
| LINUX_AMD64_REQUIRED | YES (gate) |
| LINUX_ARM64_REQUIRED | YES (gate) |
| PERFORMANCE_DELTA_REQUIRED | NO |
| ROLLBACK_METHOD | revert single commit + lockfile |
| DECISION | `UPDATE_V041` |

### bytes

| Field | Value |
|-------|-------|
| DEPENDENCY | bytes |
| CURRENT_DIRECT_VERSION | `1` (workspace) |
| CURRENT_LOCKED_VERSION | `1.12.0` |
| TARGET_VERSION | `1.12.1` |
| DIRECT_OR_TRANSITIVE | direct (core, module-api, proxy, static, fastcgi, http3, cache, …) + transitive (hyper/http/h2/h3) |
| USAGE_LOCATIONS | ~86 files; DBEX-001 shared value type |
| HOT_PATH | YES |
| SECURITY_RELEVANCE | MEDIUM (panic safety in `Box::new` path per release notes) |
| BREAKING_CHANGES | none (patch) |
| MSRV_DELTA | rust_version `1.57` — none |
| FEATURE_DELTA | none |
| TRANSITIVE_DELTA | lock bump only expected |
| PLATFORM_DELTA | none |
| CONFIG_OR_API_DELTA | none |
| TEST_PLAN | unit/integration; protocol smokes; **perf matrix** vs v0.4.0 |
| LINUX_AMD64_REQUIRED | YES |
| LINUX_ARM64_REQUIRED | YES |
| PERFORMANCE_DELTA_REQUIRED | YES |
| ROLLBACK_METHOD | revert single commit + lockfile |
| DECISION | `UPDATE_V041` |

Evidence: Bytes v1.12.1 release notes — *“Properly handle when `Box::new` panics (#837)”*.

### serde (+ serde_derive)

| Field | Value |
|-------|-------|
| DEPENDENCY | serde / serde_derive |
| CURRENT_DIRECT_VERSION | `serde = { version = "1", features = ["derive"] }` |
| CURRENT_LOCKED_VERSION | `1.0.228` / `1.0.228` |
| TARGET_VERSION | `1.0.229` / `1.0.229` (keep macros coupled) |
| DIRECT_OR_TRANSITIVE | direct |
| USAGE_LOCATIONS | ~19 crates; config IR, CLI, control surfaces |
| HOT_PATH | NO (config/control); avoid new per-request serde |
| SECURITY_RELEVANCE | MEDIUM (config parsing surface) |
| BREAKING_CHANGES | none expected within `1.0` |
| MSRV_DELTA | rust_version `1.56` — none |
| FEATURE_DELTA | monitor `serde_core` lock coupling introduced upstream |
| TRANSITIVE_DELTA | expect `serde_core = 1.0.229` if selected by resolver |
| PLATFORM_DELTA | none |
| CONFIG_OR_API_DELTA | config round-trip tests required |
| TEST_PLAN | config parse/merge tests; `cargo test` config crates; smoke reload |
| LINUX_AMD64_REQUIRED | YES |
| LINUX_ARM64_REQUIRED | YES |
| PERFORMANCE_DELTA_REQUIRED | NO |
| ROLLBACK_METHOD | revert serde group commit |
| DECISION | `UPDATE_V041` |

### serde_json

| Field | Value |
|-------|-------|
| DEPENDENCY | serde_json |
| CURRENT_DIRECT_VERSION | `1` |
| CURRENT_LOCKED_VERSION | `1.0.150` |
| TARGET_VERSION | `1.0.151` (only if intentional companion bump) |
| DIRECT_OR_TRANSITIVE | direct |
| HOT_PATH | NO |
| SECURITY_RELEVANCE | MEDIUM |
| DECISION | `DEFER_V042_PLUS` unless pulled by serde group or advisory |

### notify

| Field | Value |
|-------|-------|
| DEPENDENCY | notify |
| CURRENT_DIRECT_VERSION | `7` (workspace) |
| CURRENT_LOCKED_VERSION | `7.0.0` |
| TARGET_VERSION | `8.2.0` (latest **stable**; **not** `9.0.0-rc.*`) |
| DIRECT_OR_TRANSITIVE | direct — `exyonq-reload-runtime`, `exyonq-mod-htaccess` |
| USAGE_LOCATIONS | `config_watcher.rs`, `mod-htaccess` watcher, watcher tests |
| HOT_PATH | NO (reload/control plane) |
| SECURITY_RELEVANCE | MEDIUM (config reload triggering) |
| BREAKING_CHANGES | **YES (7→8)**: MSRV 1.77; `notify-types` 2.x; event/serialization changes; Windows notify info alignment; symlink follow config |
| MSRV_DELTA | 1.72 → 1.77 declared by crate; still ≤ toolchain 1.93.0 |
| FEATURE_DELTA | optional flume; max_user_watches warning; overflow handling fixes |
| TRANSITIVE_DELTA | replace `instant` path in notify-types with `web-time` (may help RUSTSEC-2024-0384 if notify was a consumer — verify post-lock) |
| PLATFORM_DELTA | Linux inotify overflow/watch limits behavior changes — test on both arches |
| CONFIG_OR_API_DELTA | reload watcher API usage must recompile/adapt if types moved |
| TEST_PLAN | `exyonq-reload-runtime` watcher tests; htaccess watcher tests; config reload smoke amd64+arm64 |
| LINUX_AMD64_REQUIRED | YES |
| LINUX_ARM64_REQUIRED | YES |
| PERFORMANCE_DELTA_REQUIRED | NO |
| ROLLBACK_METHOD | revert notify commit; keep workspace on `7` |
| DECISION | `UPDATE_V041` |

`notify 9.0.0-rc.*`: prerelease + rust_version `1.85` → **forbidden** for v0.4.1.

### socket2 (dual — DBEX-006)

#### socket2 @ platform-linux (0.5.x)

| Field | Value |
|-------|-------|
| DEPENDENCY | socket2 (exyonq-platform-linux) |
| CURRENT_DIRECT_VERSION | `0.5` |
| CURRENT_LOCKED_VERSION | `0.5.10` |
| TARGET_VERSION | `0.5.10` (no newer 0.5.x on crates.io recent list) |
| DIRECT_OR_TRANSITIVE | direct + transitive via `redis 0.27.6` |
| USAGE_LOCATIONS | `crates/exyonq-platform-linux/src/linux_bind.rs` |
| HOT_PATH | YES (listen/bind) |
| SECURITY_RELEVANCE | HIGH |
| BREAKING_CHANGES | Dependabot `#5` (0.5→0.6) would be a **major** for this crate pin and expands dual-version policy scope |
| DECISION | `KEEP_CURRENT_WITH_REASON` — DBEX-006 temporary dual; unification = FUTURE / not authorized by this audit; redis still pulls 0.5.x |

#### socket2 @ mod-fastcgi (0.6.x)

| Field | Value |
|-------|-------|
| DEPENDENCY | socket2 (exyonq-mod-fastcgi) |
| CURRENT_DIRECT_VERSION | `0.6` |
| CURRENT_LOCKED_VERSION | `0.6.4` |
| TARGET_VERSION | `0.6.5` **pending changelog review at implementation** (crates.io newest stable `0.6.5`, created 2026-07-13, rust_version `1.70`) |
| DIRECT_OR_TRANSITIVE | direct; also pulled by `hyper-util` |
| USAGE_LOCATIONS | `unix_connect.rs`, connect timeout tests |
| HOT_PATH | YES (FastCGI connect) |
| SECURITY_RELEVANCE | HIGH |
| BREAKING_CHANGES | expect patch-level only within 0.6; **verify** before bump |
| TEST_PLAN | FastCGI connect/timeout tests; proxy/FastCGI smokes dual-arch |
| LINUX_AMD64_REQUIRED | YES |
| LINUX_ARM64_REQUIRED | YES |
| PERFORMANCE_DELTA_REQUIRED | YES if version changes |
| ROLLBACK_METHOD | revert socket2 commit |
| DECISION | `UPDATE_V041` only after changelog/API review; else `KEEP_CURRENT_WITH_REASON` |

Dependabot PR “0.5.10 → 0.6.4” as a single unify step: **`DEFER_V042_PLUS`** (policy), not an automatic v0.4.1 accept.

### actions/cache

| Field | Value |
|-------|-------|
| DEPENDENCY | actions/cache |
| CURRENT_DIRECT_VERSION | `@v5` (floating) |
| CURRENT_LOCKED_VERSION | tag `v5` → `caa296126883cff596d87d8935842f9db880ef25` (resolved at audit time; **not** pinned in YAML) |
| TARGET_VERSION | `@v6` / release `v6.1.0` with **immutable SHA** `55cc8345863c7cc4c66a329aec7e433d2d1c52a9` + semver comment (re-resolve SHA at implement time) |
| DIRECT_OR_TRANSITIVE | CI only |
| USAGE_LOCATIONS | `ci.yml`, `security.yml` |
| HOT_PATH | NO |
| SECURITY_RELEVANCE | MEDIUM (supply chain of CI) |
| BREAKING_CHANGES | major 5→6 — review Node runner requirements in release notes at implement |
| TEST_PLAN | workflow syntax validation; dry CI on private repo when authorized |
| DECISION | `UPDATE_V041` |

### actions/setup-go

| Field | Value |
|-------|-------|
| DEPENDENCY | actions/setup-go |
| CURRENT_DIRECT_VERSION | `@v6` |
| CURRENT_LOCKED_VERSION | tag `v6` → `924ae3a1cded613372ab5595356fb5720e22ba16` (unpinned in YAML) |
| TARGET_VERSION | `@v7` / `v7.0.0` SHA `b7ad1dad31e06c5925ef5d2fc7ad053ef454303e` + comment (re-resolve at implement) |
| USAGE_LOCATIONS | `release.yml` → `go install …/nfpm@v2.41.3` |
| HOT_PATH | NO |
| SECURITY_RELEVANCE | MEDIUM |
| BREAKING_CHANGES | major 6→7 — verify Go setup inputs still match `go-version: "1.22"` |
| DECISION | `UPDATE_V041` (keep; do **not** `REMOVE_UNUSED`) |

---

## Proposed commit sequence

```text
P14V041_PROPOSED_COMMIT_SEQUENCE =

0. PROCESS (no product merge): create branch from d326b02; quarantine lab dirt
1. chore(release): workspace version 0.4.0 → 0.4.1 scaffolding (optional early or last)
2. deps(anyhow): 1.0.103 → 1.0.104
3. deps(serde): 1.0.228 → 1.0.229 (+ serde_derive / inevitable serde_core)
4. deps(bytes): 1.12.0 → 1.12.1
5. deps(socket2): fastcgi 0.6.4 → 0.6.5 ONLY if changelog-approved; else skip
6. deps(notify): 7.0.0 → 8.2.0 (+ watcher compile fixes if required)
7. ci(actions): pin + bump actions/cache v6 (isolated)
8. ci(actions): pin + bump actions/setup-go v7 (isolated)
9. ci(actions): SHA-pin remaining critical actions (no permission widening)
10. docs: changelog / deferred Dependabot closure notes
11. release: final version/metadata commit if not done in (1)

Each of 2–9 must be independently revertable / bisectable.
No mass `cargo update`.
No prereleases.
```

---

## Proposed Linux test matrix

```text
P14V041_PROPOSED_LINUX_TEST_MATRIX =

Host A = Netcup amd64
Host B = Oracle arm64
Base   = d326b02 / post each dep commit as needed

Per host:
  - cargo build --locked (dev + release as release criteria)
  - cargo nextest / unit + integration subset
  - scripts/test-tls/test-ephemeral-tls-and-scanner.sh
  - scripts/test-tls/exyonq-sec-private-material-zero.sh
  - static smoke
  - proxy smoke
  - FastCGI smoke
  - TLS smoke
  - HTTP/2 smoke
  - HTTP/3 smoke
  - config reload smoke (mandatory around notify commit)
  - OCI image smoke amd64/arm64 + multi-arch index (before close; no publish)

NOT_EVIDENCE = Docker Desktop / Mac local
```

---

## Proposed performance matrix

```text
P14V041_PROPOSED_PERFORMANCE_MATRIX =

Baseline commit = d326b02 (v0.4.0)
Compare after   = bytes bump; and socket2 bump if landed

Scenarios (minimal maturation set):
  - P1 static (bytes hot path)
  - FastCGI connect-sensitive scenario if socket2 changes
  - One shared-path protector spot-check if bytes delta non-neutral

Hosts: Netcup amd64 + Oracle arm64
Constants: same duration/connections/loadgen/config/reps
Store: raw results under .exyonq-local/ (not public claims)
Gate: investigate >3% median; also consistent smaller / p99 / errors / RSS / arch skew
```

---

## Implementation readiness

```text
P14V041_IMPLEMENTATION_READY = NO
P14V041_IMPLEMENTATION_PHASE_OPENED = NO
```

Reasons: **B1** (trees not reconciled; lab not on published HEAD) and **B2** (unclassified dirt). Scope content itself is sufficiently defined for a later authorized `P14V041` execution phase once the maintainer establishes a clean `d326b02`-based branch.

---

## Stop

```text
AUDIT_COMPLETE = YES
FILES_CHANGED_THIS_AUDIT = docs/releases/p14v041-scope-audit.md (new)
PRODUCT_CODE_CHANGED = NO
CARGO_LOCK_CHANGED = NO
SYNC_EXECUTED = NO
COMMIT = NO
PUSH = NO
TAG = NO
RELEASE = NO
DEPENDENCY_IMPLEMENTATION = NO
```

Awaiting owner decision on this proposed scope before any `P14V041` implementation phase.
