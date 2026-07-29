# P14V041 — ExyonQ v0.4.1 Private Maturation Execution

```text
DOCUMENT = docs/releases/p14v041-execution.md
PHASE = EXECUTION
P14V041_SCOPE_APPROVED = YES
P14V041_IMPLEMENTATION_PHASE_OPENED = YES
P14V041_RELEASE_AUTHORIZED = NO
P14V041_PUBLICATION_AUTHORIZED = NO
PUBLICATION_STATUS = FORBIDDEN
PUSH = NO
TAG = NO
RELEASE = NO
GHCR_PUSH = NO
LATEST_CHANGED = NO
VISIBILITY_CHANGED = NO
HISTORY_REWRITE = NO
FORCE_PUSH = NO
SIGNING = NO
```

Scope basis: `docs/releases/p14v041-scope-audit.md`  
Owner decision: `APPROVED_WITH_MANDATORY_DEPENDENCY_COMPLETION`

---

## Identity

```text
P14V041_BASE_HEAD = d326b02b4ebc3b8dd7a8a3dbe7deeab5251d4910
P14V041_BRANCH = release/p14v041
P14V041_WORKTREE = /Volumes/Lexar/Cursor/exyonq-lab-wt-p14v041
P14V041_HEAD = f9910c6 (update on each commit; see commit log)
P14V041_WORKTREE_CLEAN = YES
P14V041_AMBIENT_LAB_DIRT_IMPORTED = NO
P14V041_PREHISTCLEAN_HISTORY_IMPORTED = NO
```

Ambient lab (untouched):

```text
AMBIENT_PATH = /Volumes/Lexar/Cursor/exyonq-lab
AMBIENT_BRANCH = perf/p8o-finite-sse-batch
AMBIENT_HEAD = 9dad9587c94f3732c1c3f8bc8153ac3b26f6e0bc
AMBIENT_BACKUP_BRANCH = backup/p14v041-ambient-lab-9dad958
AMBIENT_INVENTORY = .exyonq-local/inventories/p14v041-phase0-ambient-20260728T202745Z
```

---

## Phase 0 — B1/B2

```text
P14V041_B1_STATUS = RESOLVED_VIA_CLEAN_WORKTREE
P14V041_B2_STATUS = RESOLVED_VIA_ISOLATION_AMBIENT_PRESERVED
P14V041_AMBIENT_DIRT_PRESERVED = YES
P14V041_CLEAN_WORKTREE_CREATED = YES
```

Actions taken:

1. Inventory of ambient dirty state under ambient `.exyonq-local/inventories/`.
2. Non-destructive backup branch `backup/p14v041-ambient-lab-9dad958` at ambient HEAD.
3. Ambient working tree left intact (no `git clean` / `reset --hard` / destructive rebase).
4. `git worktree add -b release/p14v041 … d326b02` → clean tree, ambient dirt not imported.

---

## Dependency status

```text
P14V041_ANYHOW_STATUS = UPDATED
  ANYHOW = 1.0.103 → 1.0.104

P14V041_SERDE_STATUS = UPDATED
  SERDE = 1.0.228 → 1.0.229
  SERDE_DERIVE = 1.0.228 → 1.0.229
  SERDE_CORE = resolved to 1.0.229
  SERDE_JSON = LEFT_AT_1.0.150 (no advisory/coupling; not auto-bumped)

P14V041_BYTES_STATUS = UPDATED
  BYTES = 1.12.0 → 1.12.1
  LOCAL_GATES = PASS
  LINUX_PERF_VS_BASELINE = PENDING

P14V041_SOCKET2_STATUS = UPDATED_DIRECT_USAGE_TO_0_6
  SOCKET2_0_6_LINE = 0.6.4 → 0.6.5 (direct product)
  PLATFORM_LINUX_DIRECT = 0.5.10 → 0.6.x (pin "0.6"; API: set_nodelay → set_tcp_nodelay)
  DBEX-006_DIRECT_PRODUCT_DUAL_VERSION = RESOLVED
  DBEX-006_TRANSITIVE_THIRD_PARTY_DUPLICATION = REMAINS (socket2 0.5.10 via redis/hyper-util graph; out of forced scope)
  P14V041_SOCKET2_PERFORMANCE_DELTA_REQUIRED = YES (PENDING dual-arch measure)

P14V041_NOTIFY_STATUS = UPDATED
  NOTIFY = 7.0.0 → 8.2.0 (no 9.0.0-rc)
  API_ADAPT = NONE_REQUIRED (Watcher/EventKind compatible)
  LOCAL_RELOAD_HTACCESS_TESTS = PASS

P14V041_ACTIONS_CACHE_STATUS = UPDATED_SHA_PINNED
  actions/cache = v5 → v6.1.0 @ 55cc8345863c7cc4c66a329aec7e433d2d1c52a9

P14V041_ACTIONS_SETUP_GO_STATUS = UPDATED_SHA_PINNED
  actions/setup-go = v6 → v7.0.0 @ b7ad1dad31e06c5925ef5d2fc7ad053ef454303e
  go-version = 1.22 retained for nfpm

P14V041_ACTIONS_FLOATING_REFS_CURRENT = (pre-pin) ALL floating tags/branches
P14V041_ACTIONS_FLOATING_REFS_FINAL = NONE (all active uses SHA-pinned)
P14V041_ACTIONS_SHA_PIN_GATE = PASS

P14V041_DEPENDENCY_COMPLETION_GATE = PENDING_LINUX_PERF
  (Rust + Actions dep work complete; dual-arch perf + accumulated gates open)
```

### Actions SHA inventory (final)

| Action | Pin | Comment |
|--------|-----|---------|
| actions/cache | `55cc8345…` | v6.1.0 |
| actions/setup-go | `b7ad1dad…` | v7.0.0 |
| actions/checkout | `9c091bb2…` | v7.0.0 |
| actions/upload-artifact | `bbbca2dd…` | v7.0.0 |
| actions/download-artifact | `70fc10c6…` | v8.0.0 |
| Swatinem/rust-cache | `f13886b9…` | v2.8.1 |
| dtolnay/rust-toolchain | `2c7215f1…` | master tip (toolchain inputs preserved) |
| taiki-e/install-action | `599db1c4…` | nextest |
| docker/login-action | `b45d80f8…` | v4.0.0 |
| docker/build-push-action | `d08e5c35…` | v7.0.0 |
| softprops/action-gh-release | `b4309332…` | v3.0.0 |

---

## Gates

```text
P14V041_BUILD_GATE = PASS_LOCAL (workspace check; linux target check for platform-linux)
P14V041_TEST_GATE = PASS_LOCAL_SCOPED (anyhow/serde/bytes/socket2/notify packages + reload/htaccess)
P14V041_SECURITY_GATE = PASS_LOCAL (cargo audit; cargo deny advisories; private-material-zero)
  AUDIT_NOTE = RUSTSEC-2025-0134 rustls-pemfile unmaintained (pre-existing deny ignore)
  INSTANT_UNMAINTAINED = REMOVED_WITH_NOTIFY_8
P14V041_NETCUP_GATE = PENDING
P14V041_ORACLE_GATE = PENDING
P14V041_PERFORMANCE_GATE = PENDING
P14V041_OCI_SMOKE_GATE = PENDING

P14V041_RELEASE_READY = NO
P14V041_RELEASE_EXECUTED = NO
```

---

## Commit log

| # | Commit | Subject |
|---|--------|---------|
| 1 | `af04416` | chore(p14v041): establish clean 0.4.0-derived execution base |
| 2 | `4cff95b` | chore(fmt): wrap ephemeral TLS assert for rustfmt gate |
| 3 | `cecf966` | deps(anyhow): update to 1.0.104 |
| 4 | `970cc70` | deps(serde): update serde family to 1.0.229 |
| 5 | `8093e86` | deps(bytes): update to 1.12.1 |
| 6 | `06c4006` | deps(socket2): update 0.6 line to 0.6.5 |
| 7 | `715014e` | deps(socket2): migrate platform-linux direct usage 0.5 → 0.6 |
| 8 | `b214223` | deps(notify): update 7.0.0 → 8.2.0 |
| 9 | `2376193` | ci(cache): update actions/cache to v6.1.0 and SHA-pin |
| 10 | `ef3cbcd` | ci(setup-go): update actions/setup-go to v7.0.0 and SHA-pin |
| 11 | `f9910c6` | ci(actions): SHA-pin remaining admitted active Actions |
| 12 | TBD | docs(release): changelog, dependency decisions and honest limitations |
| 13 | TBD | chore(version): finalize 0.4.1 metadata |

---

## Notes

- Product work only in `release/p14v041` worktree; ambient `perf/p8o-finite-sse-batch` dirt preserved and not imported.
- No global `cargo update`. Lockfile deltas are commit-scoped.
- Stop at `P14V041_RELEASE_READY = YES` for separate publication authorize.
- `PUBLICATION_STATUS = FORBIDDEN` until owner opens a later phase.
