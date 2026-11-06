#!/usr/bin/env bash
# Real clean-worktree regression for the publication gate.
#
# The test builds a disposable repository from the actual hooks and security
# scripts, installs them with the documented setup command, and then exercises
# commit, push, the literal CI scanner command and the release preflight.
# Nothing here touches the caller's index, commits or remotes.
set -euo pipefail
LC_ALL=C
export LC_ALL
set +x

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-publication-hooks-XXXXXX")"
REPO="$TMP/repo"
BARE="$TMP/remote.git"
trap 'rm -rf "$TMP"' EXIT

pem_canary() {
  printf '%s\n%s\n%s\n' \
    "-----BEGIN ""PRIVATE KEY-----" \
    "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7FAKE_NOT_A_REAL_KEY" \
    "-----END ""PRIVATE KEY-----"
}

last_log=""
expect_pass() {
  local label="$1" log
  shift
  log="$TMP/log-${label//[^A-Za-z0-9_.-]/_}"
  last_log="$log"
  if "$@" >"$log" 2>&1; then
    if ! grep -qE 'PRIVATE_MATERIAL_FINDINGS=0' "$log"; then
      echo "ERROR: clean run did not report zero findings: $label" >&2
      cat "$log" >&2
      exit 1
    fi
    echo "PASS_EXPECT_PASS $label"
  else
    echo "ERROR: expected PASS for $label" >&2
    cat "$log" >&2
    exit 1
  fi
}

expect_block() {
  local label="$1" log
  shift
  log="$TMP/log-${label//[^A-Za-z0-9_.-]/_}"
  last_log="$log"
  if "$@" >"$log" 2>&1; then
    echo "ERROR: expected BLOCK for $label" >&2
    cat "$log" >&2
    exit 1
  fi
  if ! grep -qE 'PRIVATE_MATERIAL_FINDINGS=[1-9][0-9]*' "$log"; then
    echo "ERROR: blocked run did not report positive private-material findings: $label" >&2
    cat "$log" >&2
    exit 1
  fi
  echo "PASS_EXPECT_BLOCK $label"
}

run_in_repo() {
  (cd "$REPO" && "$@")
}

# Start from a real clean clone of this repository, then apply the security
# files under review exactly as a committer would before testing the hooks.
git clone --quiet "$ROOT" "$REPO"
rm -rf "$REPO/scripts/security" "$REPO/scripts/test-tls" "$REPO/.githooks"
cp -R "$ROOT/scripts/security" "$REPO/scripts/security"
cp "$ROOT/scripts/setup-githooks.sh" "$REPO/scripts/setup-githooks.sh"
cp "$ROOT/scripts/verify-no-private-paths.sh" "$REPO/scripts/verify-no-private-paths.sh"
cp -R "$ROOT/scripts/test-tls" "$REPO/scripts/test-tls"
cp -R "$ROOT/.githooks" "$REPO/.githooks"
for rel in \
  .github/workflows/ci.yml \
  .github/workflows/security.yml \
  .github/workflows/release.yml \
  scripts/gates/pre-r5-grc-readiness-gate.sh \
  docs/security/private-material-zero.md \
  docs/releases/0.9.1-requirements-evidence.md \
  docs/architecture/070-ol03-client-throttle.md \
  docs/governance/evidence/INDEX.md; do
  mkdir -p "$REPO/$(dirname "$rel")"
  cp "$ROOT/$rel" "$REPO/$rel"
done
git -C "$REPO" remote remove origin

git -C "$REPO" init -q
git -C "$REPO" config user.name 'ExyonQ Publication Hook Test'
git -C "$REPO" config user.email 'publication-hook-test@exyonq.invalid'
# When the test runs from a committed tree, the clone plus the copied security
# files can already be clean. Only create a baseline commit when that copy
# actually changed the index; an empty commit is not a valid baseline.
if [[ -n "$(git -C "$REPO" status --porcelain)" ]]; then
  git -C "$REPO" add -A
  git -C "$REPO" commit -q --no-verify -m 'publication hook test baseline'
else
  echo 'PUBLICATION_HOOK_TEST_BASELINE=already-clean'
fi

run_in_repo bash scripts/setup-githooks.sh
[[ "$(git -C "$REPO" config --get core.hooksPath)" == ".githooks" ]] || {
  echo "ERROR: core.hooksPath was not installed" >&2
  exit 1
}
for hook in pre-commit pre-push; do
  [[ -x "$REPO/.githooks/$hook" ]] || {
    echo "ERROR: hook is not executable: $hook" >&2
    exit 1
  }
done
echo "PUBLICATION_HOOKS_INSTALLED=PASS"

# 1. Real commit hook: clean index passes, staged private canary is rejected.
printf 'clean commit\n' >"$REPO/clean-commit.txt"
git -C "$REPO" add clean-commit.txt
expect_pass commit_hook_clean git -C "$REPO" commit -m 'clean commit'

pem_canary >"$REPO/staged-canary.txt"
git -C "$REPO" add staged-canary.txt
expect_block commit_hook_canary git -C "$REPO" commit -m 'staged canary'
git -C "$REPO" reset --hard -q HEAD

# 2. Real push hook: clean commit reaches a disposable bare remote; a canary
# commit does not. --no-verify is only used to create the hostile local commit.
git init --bare -q "$BARE"
git -C "$REPO" remote add origin "$BARE"
expect_pass push_hook_clean git -C "$REPO" push -q origin HEAD:refs/heads/publication-hook-test

pem_canary >"$REPO/push-canary.txt"
git -C "$REPO" add push-canary.txt
git -C "$REPO" commit -q --no-verify -m 'push canary'
expect_block push_hook_canary git -C "$REPO" push -q origin HEAD:refs/heads/publication-hook-test
git -C "$REPO" reset --hard -q HEAD~1
rm -f "$REPO/push-canary.txt"

printf 'clean push after rejection\n' >"$REPO/clean-push.txt"
git -C "$REPO" add clean-push.txt
git -C "$REPO" commit -q -m 'clean push after rejection'
expect_pass push_hook_clean_after_rejection git -C "$REPO" push -q origin HEAD:refs/heads/publication-hook-test

# 3. Literal CI command: the same invocation used by ci.yml/security.yml.
expect_pass ci_publication_clean run_in_repo bash scripts/security/scan-private-material.sh --publication --repo .
pem_canary >"$REPO/untracked-ci-canary.txt"
expect_block ci_publication_canary run_in_repo bash scripts/security/scan-private-material.sh --publication --repo .
rm -f "$REPO/untracked-ci-canary.txt"

# 4. Literal release preflight, which also includes the scanner regression suite.
expect_pass release_preflight_clean run_in_repo bash scripts/security/private-material-release-gates.sh preflight
pem_canary >"$REPO/untracked-preflight-canary.txt"
expect_block release_preflight_canary run_in_repo bash scripts/security/private-material-release-gates.sh preflight
rm -f "$REPO/untracked-preflight-canary.txt"

# The same flow is clean again after the canary is removed.
expect_pass final_clean_publication run_in_repo bash scripts/security/scan-private-material.sh --publication --repo .

echo "PUBLICATION_HOOKS_COMMIT_GATE=PASS"
echo "PUBLICATION_HOOKS_PUSH_GATE=PASS"
echo "PUBLICATION_HOOKS_CI_ENTRYPOINT_GATE=PASS"
echo "PUBLICATION_HOOKS_RELEASE_PREFLIGHT_GATE=PASS"
echo "PUBLICATION_HOOKS_TEST=PASS"
