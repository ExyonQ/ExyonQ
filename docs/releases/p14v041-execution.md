# P14V041 — ExyonQ v0.4.1 Private Maturation Execution

```text
DOCUMENT = docs/releases/p14v041-execution.md
PHASE = CLOSE_COMPLETE
P14V041_STATUS = COMPLETE
P14V041_CLOSE_STATUS = COMPLETE
P14V041_PUBLICATION_STATUS = PRIVATE_V041_RELEASE_COMPLETE

P14V041_SCOPE_APPROVED = YES
P14V041_IMPLEMENTATION_PHASE_OPENED = YES
P14V041_RELEASE_AUTHORIZED = YES
P14V041_PRIVATE_PUBLICATION_AUTHORIZED = YES
P14V041_PUBLIC_OPENING = NO

PUSH = SOURCE_AND_SIGNED_TAG_DONE
TAG = v0.4.1
RELEASE = CREATED_PRIVATE
GHCR_PUSH = VERSIONED_0_4_1_ONLY
LATEST_CHANGED = NO
VISIBILITY_CHANGED = NO
HISTORY_REWRITE = NO
FORCE_PUSH = NO
```

Scope basis: `docs/releases/p14v041-scope-audit.md`  
Owner decisions: `APPROVED_WITH_MANDATORY_DEPENDENCY_COMPLETION` → `P14V041_CLOSE_AUTHORIZED` + `P14V041_PRIVATE_PUBLICATION_AUTHORIZED`

---

## Identity

```text
P14V041_BASE_HEAD =
  d326b02b4ebc3b8dd7a8a3dbe7deeab5251d4910

P14V041_RELEASE_HEAD =
  43805eb04a79babfbe443e2db4ca0d5b6658c80c

P14V041_FREEZE_AMENDMENT =
  owner EXPECTED_HEAD f335360 amended by publication-gate defects
  (rustfmt whitespace + release-manifest product version allowlist 0.4.1)
  → 94f29a9 → 43805eb; remote main FF only

P14V041_BRANCH = release/p14v041
P14V041_WORKTREE = /Volumes/Lexar/Cursor/exyonq-lab-wt-p14v041
P14V041_TAG = v0.4.1
P14V041_AMBIENT_LAB_DIRT_IMPORTED = NO
P14V041_PREHISTCLEAN_HISTORY_IMPORTED = NO
```

Ambient lab (untouched during close):

```text
AMBIENT_PATH = /Volumes/Lexar/Cursor/exyonq-lab
AMBIENT_BRANCH = perf/p8o-finite-sse-batch
```

---

## Publication verdicts

```text
P14V041_TAG_SIGNATURE = PASS
P14V041_TAG_SIGNER_FINGERPRINT =
  SHA256:U2Mnxtmcl1Xz7dw6lyMaLciwsBM/V/vmmDyU4iljz1c

P14V041_CHECKSUM_SIGNATURE = PASS
P14V041_COSIGN_PUBLIC_KEY_HASH =
  83931e3916b5d50b571fbc4926f7eef6ed1031b23f62ef17c1bf2780b6031562
P14V041_OFFLINE_NO_TLOG = YES

P14V041_GHCR_IMAGE = ghcr.io/exyonq/exyonq:0.4.1
P14V041_OCI_AMD64_DIGEST =
  sha256:82bcb6c2106d57f569a857ace58d24f9bcaa4106afa8bffc6160cdf028ea9ae5
P14V041_OCI_ARM64_DIGEST =
  sha256:3ff40949956809558010b15d52a698be9215699b142c232c0037324f44c75e4f
P14V041_OCI_INDEX_DIGEST =
  sha256:ac0d74f1a49c9c99b0b093e8d4eff8db72b775a6cc20bce59fa0646f496d33f5
P14V041_OCI_DUAL_ARCH = PASS
P14V041_OCI_NETCUP_DIGEST_SMOKE = PASS
P14V041_OCI_ORACLE_DIGEST_SMOKE = PASS
P14V041_OCI_SIGNATURE = PASS

P14V041_GITHUB_RELEASE = CREATED_PRIVATE
P14V041_RELEASE_URL = https://github.com/ExyonQ/ExyonQ/releases/tag/v0.4.1
P14V041_RELEASE_ASSET_VERIFY = PASS
P14V041_RELEASE_CHECKSUM_VERIFY = PASS
P14V041_REMOTE_TAG_SIGNATURE = PASS

P14V041_GHCR_VISIBILITY = PRIVATE
P14V041_GITHUB_VISIBILITY = PRIVATE

P14V041_LATEST =
  PREEXISTING_UNRELATED_PRESERVED
P14V041_LATEST_DIGEST =
  sha256:6f451ea6434f42d3e008e38adeee9025ebe9cd45734a46f736e98e4696a6759e
P14V041_LATEST_CREATED = NO
P14V041_LATEST_UPDATED = NO
P14V041_LATEST_DELETED = NO

P14V041_HISTORY_REWRITE = NO
P14V041_FORCE_PUSH = NO
P14V041_PUBLIC_OPENING = NO
P14V041_BACKUP_KEY_ACCESS = NO
```

---

## Implementation gates (pre-close)

```text
P14V041_DEPENDENCY_COMPLETION_GATE = PASS
P14V041_ACTIONS_SHA_PIN_GATE = PASS
P14V041_NETCUP_GATE = PASS
P14V041_ORACLE_GATE = PASS
P14V041_PERFORMANCE_GATE = PASS
P14V041_RELEASE_READY = YES
```

Dependency highlights:

- anyhow 1.0.104; serde 1.0.229; bytes 1.12.1
- socket2 direct line 0.6.5 (+ platform-linux 0.5→0.6); transitive 0.5.10 via redis remains
- notify 8.2.0 + config watcher path-filter fix
- Actions SHA-pinned

---

## Close commit chain (publication)

| Commit | Subject |
|--------|---------|
| `f335360` | (pre-amend tip referenced by owner EXPECTED_HEAD) |
| `94f29a9` | chore(fmt): rustfmt config_watcher |
| `43805eb` | fix(release): allow product version 0.4.1 in manifest policy |

Tag `v0.4.1` points at `43805eb` (annotated SSH-signed).

---

## Notes

- Product/publication work executed only in `release/p14v041` worktree; ambient dirt preserved.
- Official signing (tag SSH, Cosign blob, Cosign OCI) performed manually in Terminal.app; Cursor verified with public material only.
- No `latest` mutation; repository and GHCR remain private.
- This ledger update may land as a post-tag documentation commit on `main` and does not move `v0.4.1`.
