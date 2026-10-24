# ExyonQ Git Worktree Policy

```text
DOCUMENT = docs/governance/git-worktree-policy.md
RULE_ID = EXYONQ-GIT-WORKTREE-POLICY
RULE_STATUS = MANDATORY
FAIL_MODE = FAIL_CLOSED
POLICY_KIND = OPERATIONAL_CONTRACT
AGENT_RULE = .cursor/rules/exyonq-worktree-contract.mdc
RELATED = exyonq-worktree-contract | publication-privacy | 0-core-architecture | 128-phase1-reality-product-closure
```

Git worktrees are an allowed isolation mechanism. They do **not** change canonical authority and do **not** authorize sharing mutable state between concurrent executions.

```text
CANONICAL_DEVELOPMENT_TREE = /Volumes/Lexar/Cursor/exyonq-laboratorio
PUBLIC_CLOUD_AUTHORITY     = /Volumes/Lexar/Cursor/exyonq-github

WORKTREE_ROLE = ISOLATED_DEVELOPMENT_DIAGNOSTIC_EXPERIMENT_ONLY
WORKTREE_IS_PUBLICATION_AUTHORITY = NO
WORKTREE_MAY_REPLACE_EXYONQ_LABORATORIO = NO
WIP_LIMIT = 1

MUTABLE_STATE_ISOLATION_REQUIRED = YES
DEFAULT_CONTROL_SOCKET_MULTI_WORKTREE_SAFE = NO
PORT_NAMESPACE = REQUIRED_EXPLICIT
SOCKET_NAMESPACE = REQUIRED_EXPLICIT
SIBLING_WORKTREE_SCOPE_DEFAULT = OUT_OF_SCOPE

PRUNE_REQUIRES_EXPLICIT_OWNER_AUTHORIZATION = YES
PUSH_REQUIRES_EXPLICIT_OWNER_AUTHORIZATION = YES
TAG_REQUIRES_EXPLICIT_OWNER_AUTHORIZATION = YES
RELEASE_REQUIRES_EXPLICIT_OWNER_AUTHORIZATION = YES
```

`exyonq-github` is a sanitized export surface, not a git worktree of lab.

### Owner seal (2026-08-20)

```text
WORKTREE_POLICY_STATUS = CLOSED_DOCUMENTED

SQLITE_DATABASES_FOUND = []
SQLITE_ISOLATION_DEFECT = NO

DEFAULT_CONTROL_SOCKET = /tmp/exyonq.sock
DEFAULT_CONTROL_SOCKET_SCOPE = HOST_SHARED
CONCURRENT_DEFAULT_WORKTREES_SAFE = NO
COLLISION_CHECK = FAIL until explicitly isolated

P16_STALE_REGISTRATION_AUDIT = COMPLETE
P16_PRUNE_AUTHORIZED = NO
PRUNE_EXECUTED = NO

DETACHED_UNIQUE_COMMITS_PRESERVE_BEFORE_PRUNE =
  0ca00ba895132358da1297e03ab5f181bf9995ac
  9166a7a7af4bb6d65917b923f369300d993e35e2

BACKUP_REFS_CREATED_THIS_WIP = NO
```

---

## 1. Git authority

Git worktrees share the same repository / common object database.

Therefore:

- commits created in a worktree belong to the same Git repository;
- refs and branches are repository-wide;
- one branch may only be checked out in one worktree at a time;
- deleting a worktree must never be confused with deleting repository history;
- a worktree does **not** become an independent clone or publication authority.

```text
release/v0.4.4-integration remains checked out only in:
  /Volumes/Lexar/Cursor/exyonq-laboratorio

DO_NOT_CHECK_OUT_SAME_BRANCH_IN_ANOTHER_WORKTREE = YES
```

For isolated work, create a dedicated branch unless the current owner instruction specifies another:

```text
diag/<wip>
perf/<wip>
audit/<wip>
experiment/<wip>
```

Approved extra-worktree placements:

```text
<canonical>/.exyonq-local/worktrees/<name>
/Volumes/Lexar/Cursor/exyonq-laboratorio-wt-<name>   # only when the owner names that path
```

`/private/tmp` and `/tmp` are **not** approved placements for new worktrees.

---

## 2. Mutable state isolation — CRITICAL

Every worktree MUST have its own mutable runtime and diagnostic state.

Never share between concurrently active worktrees:

```text
SQLite databases / WAL / SHM
runtime databases
PID files
Unix sockets
lock files
temporary state
benchmark evidence
profiling output
coverage output
generated runtime configs
runtime logs
cache directories
FD/state registries
test runtime state
```

Do **not** rely on SQLite locking (or any lock file) as the isolation contract.

If two worktrees can resolve to the same writable SQLite database or the same writable socket/PID/lock path:

```text
STOP
ISOLATION_DEFECT = YES
CONCURRENT_EXECUTION_AUTHORIZED = NO
```

### 2.1 SQLite verification (canonical tree, 2026-08-20)

Product crates, `Cargo.toml` workspace manifests, and `Cargo.lock` contain **no** `rusqlite` / `sqlx` / `libsqlite3-sys` / `sqlite` dependency. A filesystem walk of the canonical tree (excluding `.git/` and `target/`) found **zero** `*.db` / `*.sqlite` / `*.sqlite3` / `*.wal` / `*.shm` files.

```text
SQLITE_DATABASES_FOUND = []
SQLITE_PRODUCT_STATE = NONE
CONCURRENT_MULTI_WORKTREE_SAFE = NOT_APPLICABLE  # no SQLite-backed ExyonQ state
```

If SQLite is introduced later, each database MUST be recorded here with:

```text
DATABASE_PATH =
PATH_SCOPE = WORKTREE_LOCAL | REPO_SHARED | USER_GLOBAL | HOST_SHARED | OTHER
WAL_PATH =
SHM_PATH =
CONCURRENT_MULTI_WORKTREE_SAFE = YES | NO | NOT_APPLICABLE
```

Any `PATH_SCOPE != WORKTREE_LOCAL` with write access → `STOP`.

---

## 3. `.exyonq-local`

Preferred rule: `<worktree>/.exyonq-local/` is private to that worktree.

Examples:

```text
/Volumes/Lexar/Cursor/exyonq-laboratorio/.exyonq-local/
/Volumes/Lexar/Cursor/exyonq-laboratorio/.exyonq-local/worktrees/<name>/.exyonq-local/
/Volumes/Lexar/Cursor/exyonq-laboratorio-wt-<name>/.exyonq-local/
```

No runtime or evidence state may silently resolve back to the canonical `exyonq-laboratorio/.exyonq-local` directory unless the access is explicitly read-only and named in the current task.

Nested worktrees under `.exyonq-local/worktrees/<name>` MUST still use their own nested `.exyonq-local/` for writable state. They must not write evidence, logs, PIDs, sockets, or caches into the parent canonical `.exyonq-local/` except when copying a finished artifact as an explicit, labeled handoff.

---

## 4. Build output

Default Cargo layout is **per-worktree** `target/` (`CARGO_TARGET_DIR` unset in the canonical tree on 2026-08-20).

Sharing compiled artifacts via `CARGO_TARGET_DIR` is permitted only if correctness is preserved. Concurrent diagnostics must not confuse binaries.

Every evidence run MUST bind:

```text
WORKTREE_PATH =
GIT_COMMON_DIR =
HEAD =
TREE =
BRANCH =
BINARY_PATH =
BINARY_SHA256 =
CARGO_TARGET_DIR =
TIMESTAMP =
```

Measuring a binary built from another worktree as if it belonged to the current tree is forbidden unless the run is an explicit controlled A/B comparison that records both identities.

---

## 5. Port / socket isolation

Concurrent worktrees must not silently compete for HTTP, control, metrics, or FastCGI ports, Unix sockets, or host-global temporary paths.

```text
PORT_NAMESPACE   = REQUIRED_EXPLICIT
SOCKET_NAMESPACE = REQUIRED_EXPLICIT
```

Verified host-shared defaults (canonical product, 2026-08-20):

```text
DEFAULT_CONTROL_SOCKET =
  PATH = /tmp/exyonq.sock
  SET_BY = cli/exyonq/src/main.rs (when EXYONQ_CONTROL_SOCKET unset)
  READ_BY = cli/exyonqctl DEFAULT_CONTROL_SOCKET
  PATH_SCOPE = HOST_SHARED
  CONCURRENT_MULTI_WORKTREE_SAFE = NO

EXYONQ_API_SOCKET =
  PATH_SCOPE = OPERATOR_ENV
  CONCURRENT_MULTI_WORKTREE_SAFE = NO unless distinct per worktree

HTTP listen =
  PATH_SCOPE = CONFIG
  typical local/test default = 127.0.0.1:8080
  CONCURRENT_MULTI_WORKTREE_SAFE = NO unless distinct listen per worktree
```

Before concurrent execution:

```text
PORT_NAMESPACE =
SOCKET_NAMESPACE =
COLLISION_CHECK = PASS | FAIL
```

```text
COLLISION_CHECK = FAIL
```

until each concurrent worktree/experiment has an explicit distinct listen address, control socket, API socket, and temp/evidence root.

SQLite locking is irrelevant here. The proven isolation defect for concurrent default `exyonq serve` is the host-shared control socket, not a database.

---

## 6. Evidence authority

Evidence belongs to the worktree/run that generated it.

Every evidence directory must record at least:

```text
WORKTREE_PATH
HEAD
TREE
BRANCH
BINARY_SHA256
TIMESTAMP
```

Never use evidence generated by another worktree as evidence for the current tree unless explicitly performing a controlled A/B comparison that names both trees, both HEADs, and both `BINARY_SHA256` values.

---

## 7. WIP=1 exception

Git worktrees do **not** automatically authorize multiple product WIPs.

```text
WIP_LIMIT = 1
```

Parallel worktrees are allowed for:

- read-only audit;
- controlled A/B;
- reproductions;
- isolated diagnostic experiments;
- explicitly authorized parallel branches.

They are **not** permission to start another independent product capability or optimization.

---

## 8. Agent scope

An agent may access only the worktree explicitly named in its current task.

Sibling worktrees are `OUT_OF_SCOPE` by default.

No agent may inspect, modify, clean, commit, reset, stash, rebase, or delete a sibling worktree unless explicitly authorized in the current message.

---

## 9. Cleanup

Stale registrations may be audited. They must **not** be pruned or deleted without a later explicit owner authorization in the current message.

```text
P16_STALE_REGISTRATION_AUDIT = COMPLETE
P16_PRUNE_AUTHORIZED = NO
PRUNE_EXECUTED = NO
```

Full inventory (local, not a public document):

```text
.exyonq-local/status/git-worktree-p16-audit.json
```

`git worktree prune` removes stale administrative files. It does **not** delete named branches. Detached HEADs whose commits are not reachable from any remaining ref become dangling after prune.

---

## 10. Push / publication

```text
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION = NO
SYNC_TO_GITHUB = NO
```

A worktree never implies permission to sync to `exyonq-github`. Local worktree paths (`/Volumes/Lexar/Cursor/exyonq-*`, `exyonq-laboratorio-wt-*`, `/private/tmp/exyonq-*`) must not be published.

---

## Precedence

On conflict with convenience (shared `target/`, shared `/tmp` sockets, shared `.exyonq-local`), **this policy wins**.

Canonical lab authority, publication privacy, private-material, and integrity/evidence rules still win in their own domains. A worktree does not relax `COMMIT` / `PUSH` / `TAG` defaults.
