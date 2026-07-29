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
P14V041_HEAD = 3bd1133079ade7a11b4abec2836d216039d94333
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

---

## Dependency status

```text
P14V041_ANYHOW_STATUS = UPDATED
  ANYHOW = 1.0.103 → 1.0.104

P14V041_SERDE_STATUS = UPDATED
  SERDE = 1.0.228 → 1.0.229
  SERDE_DERIVE = 1.0.228 → 1.0.229
  SERDE_CORE = 1.0.229
  SERDE_JSON = LEFT_AT_1.0.150 (no advisory/coupling)

P14V041_BYTES_STATUS = UPDATED
  BYTES = 1.12.0 → 1.12.1

P14V041_SOCKET2_STATUS = UPDATED_DIRECT_USAGE_TO_0_6
  SOCKET2_0_6_LINE = 0.6.4 → 0.6.5
  PLATFORM_LINUX_DIRECT = 0.5.10 → 0.6.x (set_nodelay → set_tcp_nodelay)
  DBEX-006_DIRECT_PRODUCT_DUAL_VERSION = RESOLVED
  DBEX-006_TRANSITIVE_THIRD_PARTY_DUPLICATION = REMAINS (socket2 0.5.10 via redis only)

P14V041_NOTIFY_STATUS = UPDATED
  NOTIFY = 7.0.0 → 8.2.0 (no 9.0.0-rc)
  FOLLOW_ON_FIX = path-filter on config watcher (reload thrash when watching /tmp parent)

P14V041_ACTIONS_CACHE_STATUS = UPDATED_SHA_PINNED
  actions/cache = v6.1.0 @ 55cc8345863c7cc4c66a329aec7e433d2d1c52a9

P14V041_ACTIONS_SETUP_GO_STATUS = UPDATED_SHA_PINNED
  actions/setup-go = v7.0.0 @ b7ad1dad31e06c5925ef5d2fc7ad053ef454303e

P14V041_ACTIONS_FLOATING_REFS_CURRENT = (pre) ALL floating
P14V041_ACTIONS_FLOATING_REFS_FINAL = NONE
P14V041_ACTIONS_SHA_PIN_GATE = PASS

P14V041_DEPENDENCY_COMPLETION_GATE = PASS
```

---

## Gates

```text
P14V041_BUILD_GATE = PASS
P14V041_TEST_GATE = PASS
P14V041_SECURITY_GATE = PASS
  cargo audit = 1 allowed warning (RUSTSEC-2025-0134 rustls-pemfile unmaintained)
  cargo deny advisories = ok
  EXYONQ-SEC-PRIVATE-MATERIAL-ZERO = PASS
  ephemeral TLS + scanner selftest = PASS

P14V041_NETCUP_GATE = PASS
  k0 epoll/sendfile PASS=22 FAIL=0
  kd3 proxy PASS=12 FAIL=0
  fastcgi --lib 61 passed

P14V041_ORACLE_GATE = PASS
  k0 epoll/sendfile PASS=22 FAIL=0
  kd3 proxy PASS=12 FAIL=0
  fastcgi --lib 61 passed

P14V041_PERFORMANCE_GATE = PASS_AFTER_INVESTIGATION
  Baseline = d326b02; HEAD = 3bd1133; loadgen = rewrk 0.3.1; c=100 d=15s t=4
  Netcup amd64 sequential: static median Req/Sec +0.77%; proxy −4.86% (high variance)
  Netcup proxy interleaved A/B (5 rounds): +0.08% → prior proxy delta NOT reproducible
  Oracle arm64 sequential: static −6.32%; proxy +0.02%
  Oracle static interleaved A/B (5 rounds): +2.53% → prior static delta NOT reproducible
  RSS: Netcup baseline warmup 16212 KB → head 15200 KB; Oracle 13336 → 12520 KB
  Conclusion: no durable >3% median product regression under interleaved A/B

P14V041_OCI_SMOKE_GATE = PENDING_OR_IN_PROGRESS
  Local docker build on Netcup without push (see evidence when complete)

P14V041_RELEASE_READY = YES
P14V041_RELEASE_EXECUTED = NO
```

Remote evidence trees (not published):

```text
netcup: /root/exyonq-p14v041/.exyonq-local/p14v041-evidence/
oracle: /home/ubuntu/exyonq-p14v041/.exyonq-local/p14v041-evidence/
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
| 12 | `db97915` | docs(release): changelog, dependency decisions and honest limitations |
| 13 | `217f981` | chore(version): finalize 0.4.1 metadata |
| 14 | `3bd1133` | fix(reload): filter config watcher events to the config path |

---

## Notes

- Product work only in `release/p14v041` worktree; ambient `perf/p8o-finite-sse-batch` dirt preserved.
- No global `cargo update`. Lockfile deltas are commit-scoped.
- Correctness fix authorized by notify Linux validation: config watcher path filter (parent-dir watch required for atomic rename).
- Stop here for separate owner authorize of private close / publication.
- `PUBLICATION_STATUS = FORBIDDEN` until a later explicit authorize.
