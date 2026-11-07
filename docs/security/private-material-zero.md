# EXYONQ-SEC-PRIVATE-MATERIAL-ZERO

```text
POLICY_ID = EXYONQ-SEC-PRIVATE-MATERIAL-ZERO
POLICY_STATUS = MANDATORY
FAIL_MODE = FAIL_CLOSED
CANONICAL_SCANNER = scripts/security/scan-private-material.sh
LOCAL_GATE = scripts/security/private-material-release-gates.sh
CURSOR_RULE = .cursor/rules/exyonq-sec-private-material-zero.mdc
```

## Ban

Private-key-formatted material, signing secrets, PATs, and prohibited private
signing-volume paths must not enter the repository, source archives, binary
packages, OCI images, or release assets — including fixtures labeled test/demo.

## Canonical scanner

```bash
bash scripts/security/scan-private-material.sh --selftest
bash scripts/security/scan-private-material.sh --publication --repo .
bash scripts/security/scan-private-material.sh --git-index --repo .
bash scripts/security/scan-private-material.sh --history --repo .
```

Engine: `scripts/security/lib/private_material_scan.py`  
Related helpers: `scripts/release/scan-private-keys.sh`, `scripts/test-tls/exyonq-sec-private-material-zero.sh` (must not diverge from FAIL_CLOSED semantics).

## Enforcement

| Surface | Mechanism |
| --- | --- |
| Local pre-commit | `.githooks/pre-commit` → `--git-index` (staged index blobs) |
| Local pre-push | `.githooks/pre-push` → `--publication` + `verify-no-private-paths` |
| CI | `security.yml` private-material job |
| Release | `private-material-release-gates.sh` + release checklist |

Install the local hooks with `bash scripts/setup-githooks.sh`. The hook files are
tracked; a clone without `core.hooksPath=.githooks` is not locally protected
until the setup command is run, but CI and the release preflight still enforce
the same gate.

## Publication set

`--publication` is the release/push review mode. It enumerates
`git ls-files --cached --others --exclude-standard`: tracked working-tree files
plus untracked files that are not ignored. It also reads the exact blob hashes
from the index, so a secret staged in the index is caught even when the working
tree is clean. The legacy tracked-only mode only enumerates tracked working-tree
paths and therefore cannot see either an untracked canary that a normal
`git add -A` would publish or a staged blob that differs from the worktree. It
is diagnostic-only and requires the explicit `--allow-blind-scan` risk marker;
the wrapper rejects it by default.

`--history` is a read-only scan of every commit and unique blob reachable from
all refs, including paths deleted in later commits. It reuses the same content,
archive and binary-string rules and reports `HISTORY_COMMITS`, `HISTORY_BLOBS`,
`HISTORY_EXPOSURE` and `HISTORY_ROTATION_REQUIRED` without printing secret
bodies. It never rewrites history. Hits that are only grep/regex definitions or
explicit redaction/test bearer fixtures are reported separately as
`HISTORY_FALSE_POSITIVES`; any other hit remains blocking.

`scripts/security/tests/test-publication-path-guard.sh` fails if a current
workflow, publication script or current documentation reintroduces the legacy
tracked-only invocation outside its diagnostic allowlist. `scripts/setup-githooks.sh`
installs both real hooks and the clean-worktree test at
`scripts/security/tests/test-publication-hooks.sh` exercises commit and push
rejection through the actual hook files.
Local-only directories are kept out of the set with ignore rules (`.agents/`,
`.cloud-attachments/`, `target-linux/`); they are not release material and must
not be forced into a bundle.

Private signing keys remain **outside** the repository and outside agent access.
TLS tests use ephemeral generation only (`scripts/test-tls/generate-ephemeral-tls.sh`).
