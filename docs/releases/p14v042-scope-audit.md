# P14V042-SCOPE — ExyonQ v0.4.2 Scope Audit

```text
DOCUMENT = docs/releases/p14v042-scope-audit.md
PHASE = SCOPE_AUDIT_ONLY
P14V042_AUDIT_STATUS = COMPLETE_PROPOSAL
P14V042_SCOPE_AUDIT_AUTHORIZED = YES
P14V042_IMPLEMENTATION_AUTHORIZED = NO
P14V042_IMPLEMENTATION_OPENED = NO
P14V042_RELEASE_AUTHORIZED = NO

P14V042_BASE_HEAD = 25dcb4e9b323bbb2163c5f28f600175bfe725295
P14V042_BRANCH = audit/p14v042-scope
P14V042_WORKTREE = exyonq-lab-wt-p14v042-scope
P14V042_TREE_CLEAN = YES

P14V042_PROPOSED_KIND =
  PRIVATE_CORRECTNESS_SECURITY_MAINTENANCE_RELEASE

PUSH = NO
TAG = NO
RELEASE = NO
GHCR_PUSH = NO
LATEST_CHANGED = NO
VISIBILITY_CHANGED = NO
HISTORY_REWRITE = NO
FORCE_PUSH = NO
```

Prior proposal (preserved, not rewritten as authority): `docs/releases/p14v042-scope-proposal.md`  
Owner decision on proposal: `ACCEPTED_WITH_WORKTREE_CORRECTION`  
Canonical closed release: `v0.4.1` → `43805eb` (tag); ledger tip `25dcb4e` (docs only; **do not retarget tag**).

Historical worktree (do not use for future product work; retain until owner confirms dispose):

```text
P14V041_WORKTREE = exyonq-lab-wt-p14v041
P14V041_TRACKED_TREE_CLEAN = YES
P14V041_PROPOSAL_PRESERVED_SHA256 =
  d0da05b6e288443a537290f0617a8972efa077b190897fb3d1ecc6adfa4317cf
P14V041_REMOTE_V041_RECOVERABLE = YES
  origin/main = 25dcb4e…
  refs/tags/v0.4.1^{} = 43805eb…
P14V041_EXCLUSIVE_LOCAL_EVIDENCE = YES (.exyonq-local/p14v041-close/* — release packet, digests, manual sign scripts)
P14V041_WORKTREE_DELETE_AUTHORIZED = NO (await owner)
```

---

## Executive summary

```text
P14V042_PROPOSED_SCOPE =
  Private patch release focused on correctness metadata, living-doc honesty,
  rustls-pemfile unmaintained debt, low-risk serde_json bump, and a
  justified redis→socket2 0.5 residual plan — without public opening or
  speculative product features.

P14V042_PRIORITIES =
  P0: source_revision OCI/binary coherence + fail-closed release gate;
      living CHANGELOG/README honesty for private v0.4.1;
      rustls-pemfile removal via rustls-pki-types PEM APIs (preferred).
  P1: serde_json 1.0.150 → 1.0.151; Dependabot triage; redis upgrade study
      to drop transitive socket2 0.5.10.
  P2: only reproducible ops/correctness defects; supply-chain harden without
      permission widening; perf only with hypothesis + dual-arch evidence.

P14V042_BLOCKERS =
  - Owner accept of this audit before any implementation authorize.
  - Redis major/minor path choice (0.32.x vs 1.x) before coding socket2 residual removal.
  - No implementation / push / tag / release until express authorize.
```

---

## A. Source revision (P0)

### Mechanism

`cli/exyonq/build.rs` / `cli/exyonqctl/build.rs` embed `EXYONQ_SOURCE_REVISION` via:

1. env `EXYONQ_SOURCE_REVISION` if set non-empty; else  
2. `git rev-parse HEAD`; else  
3. literal `unknown`.

`exyonq --version` prints that env via `env!("EXYONQ_SOURCE_REVISION")`.

WS6 (`p15-ws6-build-artifacts.sh`) records HEAD in **build-manifest.json** but does **not** export `EXYONQ_SOURCE_REVISION` before `cargo build`. When `.git` is present, build.rs still resolves HEAD.

Dockerfile sets `ARG EXYONQ_GIT_REVISION=unknown` for **OCI labels only**. Builder `RUN cargo build` does **not** receive `EXYONQ_SOURCE_REVISION`. `.dockerignore` excludes `.git` → build.rs cannot git-resolve → binary embeds `unknown`.

### Measured matrix (v0.4.1)

| Surface | Result |
|---------|--------|
| `P14V042_SOURCE_REVISION_LOCAL_RELEASE` | **PASS** when built in a git checkout (build.rs → HEAD). Not separately re-built in this audit. |
| `P14V042_SOURCE_REVISION_PACKAGED_BINARY` | **PASS** — amd64 + arm64 `exyonq-0.4.1-linux-*.tar.gz` contain `source_revision=43805eb04a79babfbe443e2db4ca0d5b6658c80c` (string extract). |
| `P14V042_SOURCE_REVISION_OCI_AMD64` | **FAIL** — image smoke: `source_revision=unknown` (Netcup digest smoke). |
| `P14V042_SOURCE_REVISION_OCI_ARM64` | **FAIL** — same pattern on Oracle (index multi-arch; binary built without git/env). |

OCI **labels** can be correct when `--build-arg EXYONQ_GIT_REVISION=<sha>` is passed (done for 0.4.1), while **`--version` binary metadata remains `unknown`** → label/binary incoherence.

### Design requirements (implementation phase — not done now)

```text
- Pass revision explicitly into cargo (ENV EXYONQ_SOURCE_REVISION) and OCI ARG.
- Do not rely on .git inside Docker context; keep .git in .dockerignore.
- Same SHA for amd64 and arm64 official artifacts.
- Labels org.opencontainers.image.revision and --version source_revision must match.
- No local paths / ambient dirt in metadata.
- Proposed gate:
  RELEASE_BUILD_SOURCE_REVISION_UNKNOWN = FAIL_CLOSED
  (official release / OCI / packaging reject unknown)
```

### Proposed work

```text
P14V042_SOURCE_REVISION_WORK =
  INCLUDE_V042
  - Dockerfile: ARG/ENV EXYONQ_SOURCE_REVISION required for builder stage;
    wire to cargo; fail build if empty/unknown for release profile.
  - p15-ws6-build-artifacts.sh: export EXYONQ_SOURCE_REVISION=$HEAD before cargo.
  - release.yml / dual-arch scripts: same explicit env.
  - Verification: strings/--version + OCI label equality dual-arch.
  - Gate script or CI check: FAIL_CLOSED on unknown for release artifacts.
```

---

## B. Documentary honesty (P0)

### Classification

| Document | Class | Disposition |
|----------|-------|-------------|
| `docs/releases/p14v041-scope-audit.md` | **Frozen historical** (`PUBLICATION_STATUS=FORBIDDEN` at audit time) | **Do not rewrite** to pretend it knew the close outcome. |
| `docs/releases/p14v041-execution.md` / `p14v041-close.md` | **Frozen close ledgers** (`PRIVATE_V041_RELEASE_COMPLETE`) | **Do not rewrite** history; already accurate. |
| `CHANGELOG.md` § `[0.4.1]` | **Living** | Claims `not published`, `P14V041_RELEASE_EXECUTED=NO`, `PUBLICATION_STATUS=FORBIDDEN` — **stale** vs private release. Update to private-published facts without erasing that maturation phase started unpublished. |
| `README.md` | **Living** | Still says current version **0.4.0** — **stale**. |
| `security/signing/README.md` | **Living / partially stale** | Still says `v0.4.0` does not exist yet — historical template; refresh carefully or mark superseded by TRUST-POLICY + actual tags. |
| `packaging/docker/Dockerfile` `ARG EXYONQ_VERSION=0.4.1` | Living packaging default | Keep in sync with workspace version when bumping to 0.4.2. |

```text
P14V042_HONESTY_WORK =
  INCLUDE_V042 (docs-only commits)
  - CHANGELOG: record private v0.4.1 publication (tag, GHCR :0.4.1, Release URL);
    keep an honest note that public opening remains deferred.
  - README: current product version → 0.4.1 (then 0.4.2 when releasing).
  - Optional: security/signing README operator examples → v0.4.1 without claiming public trust root published.
  EXCLUDE: rewriting frozen P14V041 scope/execution/close ledgers.
```

---

## C. rustls-pemfile / RUSTSEC-2025-0134 (P0)

### Inventory

| Field | Value |
|-------|-------|
| Locked | `rustls-pemfile 2.2.0` (crates.io max stable still 2.2.0 — no newer fix) |
| Direct | **YES** — workspace `rustls-pemfile = "2"`; `exyonq-mod-tls`; also `core` (tests) |
| Usage | PEM load of certs / PKCS#8 keys in `crates/exyonq-mod-tls/src/lib.rs` (`certs`, `pkcs8_private_keys`) → `rustls::pki_types::{CertificateDer,PrivateKeyDer}` |
| Untrusted input | Operator-configured cert/key **paths** (filesystem). Not raw client-supplied PEM on the request hot path. Still security-sensitive (TLS identity material). |
| Advisory | Unmaintained (`RUSTSEC-2025-0134`); not a CVSS remote RCE by itself |

### Options

| Option | Verdict |
|--------|---------|
| **A** Migrate PEM parsing to `rustls-pki-types` / current rustls-supported PEM helpers | **PREFERRED** — removes unmaintained direct dep; aligns with already-used `pki_types` |
| **B** Update “owning” crate only | N/A as primary — ExyonQ **is** the direct owner; bumping 2.2.0 does not clear unmaintained |
| **C** Keep temporary with technical lock | Only if A blocked by API/MSRV; requires dated waiver + owner accept — **not default** |

```text
P14V042_RUSTLS_PEMFILE_STATUS_TARGET =
  REMOVE_UNMAINTAINED_DEPENDENCY

P14V042_RUSTLS_PEMFILE_WORK =
  INCLUDE_V042
  - Implement OPTION_A in exyonq-mod-tls (+ test helpers in core tests).
  - Drop workspace/direct rustls-pemfile after lock proves absence.
  - cargo audit must no longer report RUSTSEC-2025-0134 from this edge.
  - Dual-arch TLS smoke (Netcup/Oracle) + existing ephemeral TLS / PMZ gates.
  - Document API delta + MSRV (must remain ≤ toolchain 1.93.0).
```

---

## D. serde_json (P1)

| Field | Value |
|-------|-------|
| Locked | `1.0.150` |
| Latest stable (crates.io audit time) | `1.0.151` |
| Workspace | `serde_json = "1"` |
| Nature | Patch within 1.0.x |

```text
P14V042_SERDE_JSON_DECISION = UPDATE
  (to 1.0.151 unless changelog/tests show regression)

P14V042_SERDE_JSON_WORK =
  INCLUDE_V042
  - Isolated lock bump; cargo test --workspace; config/IR/JSON surfaces.
  - No MSRV elevation expected for patch.
  ROLLBACK = revert lock commit.
```

Final pin confirmed at implementation by reading 1.0.151 changelog before merge.

---

## E. Dependabot / pending dependency matrix

```text
P14V042_DEPENDENCY_MATRIX
```

| DEPENDENCY | CURRENT | LATEST_STABLE | DIRECT/TRANSITIVE | USAGE | HOT_PATH | BREAKING | MSRV_DELTA | SECURITY | RUNTIME | DECISION |
|------------|---------|---------------|-------------------|-------|----------|----------|------------|----------|---------|----------|
| serde_json | 1.0.150 | 1.0.151 | direct (workspace) | config/IR/JSON | control-plane / some request | patch | none expected | low | yes | **INCLUDE_V042** |
| rustls-pemfile | 2.2.0 | 2.2.0 | direct | TLS PEM load | startup/reload TLS | remove | check pki APIs | **unmaintained** | yes | **INCLUDE_V042** (remove) |
| rustls | 0.23.40 | 0.23.42 | direct/transitive TLS stack | TLS | handshake | patch | none expected | yes | yes | **DEFER_WITH_REASON** unless pulled by pemfile/TLS work; separate commit + dual-arch TLS smoke |
| redis | 0.27.6 | 1.4.1 (also 0.32.x on 0.x line) | direct via `exyonq-cache-redis` | cache Redis | control/cache path | **yes** (0.27→0.32 or →1.x) | verify | supply-chain + socket2 | conditional | **INCLUDE_V042** as *study+conditional migrate* (see §F) |
| socket2 | 0.6.5 + **0.5.10** | 0.6.5 | 0.6 direct; 0.5 via redis | OS sockets | yes (0.6); 0.5 transitive | n/a | — | dual-version residual | yes | Goal: remove 0.5 **if** redis migration safe — not blind unify |
| matchit | 0.8.6 (`exyonq-runtime-plan`) | 0.9.2 | direct | routing plan | possible | **major 0.8→0.9** | check | low | yes | **DEFER_WITH_REASON** (routing major; not maintenance-min) |
| memmap2 | 0.9.11 | 0.9.11 | direct (static) | static file map | static path | none | — | low | yes | **DEFER_WITH_REASON** (already current) |
| quinn | 0.11.11 | 0.11.11 | optional H3 legacy | optional provider | H3 when enabled | none | — | protocol | optional | **DEFER_WITH_REASON** (current; no drive-by) |
| wasmtime | =45.0.2 | 47.0.2 | direct pin | WASM host | off hot path default | **major** | ABI/strategic | supply-chain | leaf | **DEFER_WITH_REASON** / rule 120 — not a drive-by 0.4.2 |
| criterion | 0.5.1 | 0.8.2 | likely dev/bench | benches | no | major | — | low | no | **DEFER_WITH_REASON** (dev-only; not release blocker) |
| io-uring | (platform / optional; lock naming varies) | 0.7.13 | platform-linux area | epoll/io_uring | **hot** | possible | — | reliability | yes | **BLOCKED** / **DEFER_WITH_REASON** without dedicated dual-arch + freeze policy |
| notify | 8.2.0 | 8.2.0 (+ 9-rc forbidden) | direct | reload watcher | control | 9-rc forbidden | — | — | yes | Keep 8.2.0; **no 9-rc** |
| Actions (remaining) | SHA-pinned in 0.4.1 | n/a | CI | CI | no | pin hygiene | — | yes | no | **INCLUDE_V042** only for security/pin drift fixes; no permission widening |

Do **not** turn 0.4.2 into a full graph upgrade.

---

## F. redis → socket2 0.5.10

```text
P14V042_SOCKET2_TRANSITIVE_GOAL =
  REMOVE_0_5_IF_REDIS_MIGRATION_IS_SAFE_AND_JUSTIFIED
```

Facts:

- Direct ExyonQ `socket2` is **0.6.5** (v0.4.1). Residual **0.5.10** is **transitive via `redis 0.27.6`** (`exyonq-cache-redis`).
- crates.io: `redis 0.27.6` → `socket2 ^0.5`; **`redis 0.32.7` and `redis 1.x` → `socket2 ^0.6`**.

Allowed outcomes after implementation analysis:

```text
REMOVED_VIA_REDIS_UPDATE
REMAINS_TRANSITIVE_WITH_TECHNICAL_REASON
BLOCKED_BY_REDIS_MAJOR_MIGRATION
BLOCKED_BY_PRODUCT_REGRESSION
```

Forbidden: `DEFERRED_WITHOUT_ANALYSIS`.

```text
P14V042_REDIS_SOCKET2_WORK =
  INCLUDE_V042 (analysis + conditional code)
  Preferred study order:
    1) redis 0.27.6 → latest 0.32.x (socket2 ^0.6) with API/feature audit
       (streams/script features used by exyonq-cache-redis);
    2) only if necessary, evaluate redis 1.x as separate major.
  Prove Cargo.lock drops socket2 0.5.x entirely.
  Dual-arch cache/redis tests + no new dual-version regressions.
  Not a goal to “unify at any cost”.
```

---

## G. Security (cross-cutting)

```text
EXYONQ-SEC-PRIVATE-MATERIAL-ZERO = FAIL_CLOSED (retain)
ACTIONS_SHA_PIN_GATE = REQUIRED (retain)
```

```text
P14V042_SECURITY_WORK =
  INCLUDE_V042
  - rustls-pemfile removal (P0).
  - cargo audit / deny on every dep commit; no indefinite ignore of RUSTSEC-2025-0134 without dated waiver.
  - OCI build: keep .git out of context; explicit revision env; scan release assets / OCI FS for private PEM.
  - Workflow permission audit: no widening.
  - SBOM/notices regeneration when deps change.
  EXCLUDE: private key / backup volume access; public opening; latest mutation.
```

---

## H. Correctness candidates

```text
P14V042_CORRECTNESS_WORK =
  INCLUDE_V042
  - source_revision unknown in OCI (P0) — reproducible, demonstrated.
  - metadata coherence (--version vs OCI labels).
  DEFER: large product features; speculative refactors.
  Admit later only if newly demonstrated: reload/startup packaging TLS FastCGI proxy H2/H3 arch skew panics leaks.
```

---

## I. Performance

```text
P14V042_PERFORMANCE_WORK =
  DEFER_DEFAULT
  Open only with HYPOTHESIS + BASELINE + AFFECTED_PATH + MEASUREMENT_PLAN
  + EXPECTED_EFFECT + ROLLBACK_PLAN on Netcup amd64 + Oracle arm64.
  Mac M2 = LOCAL_ITERATION_ONLY / NOT_LINUX_EVIDENCE.
```

No general optimization track in 0.4.2.

---

## Included vs deferred

```text
P14V042_INCLUDED =
  P0 source_revision explicit wiring + FAIL_CLOSED release gate
  P0 living CHANGELOG + README honesty for private v0.4.1
  P0 rustls-pemfile → remove via OPTION_A (pki-types PEM)
  P1 serde_json → 1.0.151 (confirm changelog at impl)
  P1 redis/socket2 residual: analysis + conditional migrate to drop 0.5.10
  P1 Actions pin drift fixes only (no permission widen)
  Version bump 0.4.1 → 0.4.2 scaffolding when implementation opens

P14V042_DEFERRED =
  WAF; Nexus Panel; public admin API; multinode
  Public opening; latest create/update/delete
  Major HTTP/3 redesign; global QUIC substitution
  Full .htaccess; WordPress Enterprise
  Kernel redesign; intergenerational FastCGI pool reuse
  Speculative abstractions / unprofiled optimization
  notify 9.0.0-rc.*
  matchit 0.9; criterion 0.8; wasmtime 46/47 drive-by
  io-uring churn without dedicated freeze
  Blind rustls/quinn bumps without TLS/H3 evidence
  Reopening P14V040 / P14HISTCLEAN / P14TLSFIX / P14V041 close/sign
```

---

## Proposed commit sequence (implementation phase only)

```text
P14V042_PROPOSED_COMMIT_SEQUENCE =
  1. docs(honesty): CHANGELOG + README for private v0.4.1 (no ledger rewrite)
  2. build(meta): EXYONQ_SOURCE_REVISION fail-closed for release/OCI/WS6
  3. security(tls): remove rustls-pemfile via pki-types PEM migration
  4. deps(serde_json): 1.0.150 → 1.0.151
  5. deps(redis): conditional 0.27.6 → 0.32.x (or documented BLOCKED_* outcome)
  6. chore(version): 0.4.2 metadata / Dockerfile ARG when ready
  7. ci: only SHA-pin drift fixes if needed
```

One concern per commit; no global `cargo update`.

---

## Linux matrix (when implementation authorized)

```text
P14V042_PROPOSED_LINUX_MATRIX =
  Netcup amd64 + Oracle arm64:
  - cargo fmt/check/test; audit; deny; PMZ; ephemeral TLS
  - TLS smoke after pemfile migration
  - WS6 packaged --version source_revision == freeze SHA
  - OCI build (no latest): --version + label revision match; unknown FAIL
  - If redis migrates: cache-redis tests + socket2 0.5 absent from lock
  - Perf: only if a P2 hypothesis opens; else protector spot-check if shared paths change
```

---

## Release criteria (future close — not authorized now)

```text
P14V042_RELEASE_CRITERIA =
  - P14V042_IMPLEMENTATION authorized + complete
  - All INCLUDED items closed or explicitly BLOCKED_* with owner note
  - Dual-arch gates PASS; PMZ PASS; no Critical/High security open
  - source_revision != unknown on official amd64/arm64 + OCI
  - PRIVATE publication only if later close authorize
  - latest untouched; visibility private; no force-push / history rewrite
```

---

## Readiness

```text
P14V042_IMPLEMENTATION_READY = YES
  (audit complete; P0/P1 scope coherent; redis path still chooses 0.32.x vs block at impl)

P14V042_IMPLEMENTATION_OPENED = NO

STOP = YES
```

No dependency updates, product code edits, commits, or pushes were performed under this authorize.

---

## Owner decision requested

Accept / amend this audit, then either:

1. Authorize **`P14V042` implementation** on `audit/p14v042-scope` (or a new `release/p14v042` branch), still `PUBLICATION_STATUS=FORBIDDEN` until close authorize; or  
2. Request audit revisions; or  
3. Defer opening.

Also confirm when `exyonq-lab-wt-p14v041` may be removed (after acknowledging exclusive `.exyonq-local/p14v041-close` evidence retention/archive).
