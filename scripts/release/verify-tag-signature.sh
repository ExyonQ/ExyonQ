#!/usr/bin/env bash
# Verify SSH-signed annotated release tag without mutating git config.
set -euo pipefail
LC_ALL=C
export LC_ALL

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/release/lib/p14sign-common.sh
source "$SCRIPT_DIR/lib/p14sign-common.sh"

usage() {
  cat <<'EOF'
Usage:
  verify-tag-signature.sh \
    --repo PATH \
    --tag TAG \
    --expected-version VERSION \
    --expected-commit SHA40 \
    --allowed-signers PATH \
    [--test-fixture]

Uses temporary git -c overrides only (does not write git config).
Rejects lightweight tags, legacy versions, and commit/tag mismatch.

Without --test-fixture: TAG must equal v${VERSION} (production contract).
With --test-fixture: TAG must equal p14sign-test-only-v${VERSION}
  (TEST_ONLY / NON_PRODUCTION / DO_NOT_PUBLISH).
EOF
}

REPO=""
TAG=""
EXPECTED_VERSION=""
EXPECTED_COMMIT=""
ALLOWED_SIGNERS=""
TEST_FIXTURE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo) p14sign_require_arg --repo "${2:-}"; REPO="$2"; shift 2 ;;
    --tag) p14sign_require_arg --tag "${2:-}"; TAG="$2"; shift 2 ;;
    --expected-version) p14sign_require_arg --expected-version "${2:-}"; EXPECTED_VERSION="$2"; shift 2 ;;
    --expected-commit) p14sign_require_arg --expected-commit "${2:-}"; EXPECTED_COMMIT="$2"; shift 2 ;;
    --allowed-signers) p14sign_require_arg --allowed-signers "${2:-}"; ALLOWED_SIGNERS="$2"; shift 2 ;;
    --test-fixture) TEST_FIXTURE=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) p14sign_die "unknown argument: $1" ;;
  esac
done

p14sign_require_arg --repo "$REPO"
p14sign_require_arg --tag "$TAG"
p14sign_require_arg --expected-version "$EXPECTED_VERSION"
p14sign_require_arg --expected-commit "$EXPECTED_COMMIT"
p14sign_require_arg --allowed-signers "$ALLOWED_SIGNERS"

[[ -d "$REPO/.git" || -d "$REPO" ]] || p14sign_die "repo not found: $REPO"
[[ -f "$ALLOWED_SIGNERS" ]] || p14sign_die "allowed-signers not found: $ALLOWED_SIGNERS"
p14sign_reject_symlink "$ALLOWED_SIGNERS"
p14sign_reject_private_key_arg --allowed-signers "$ALLOWED_SIGNERS"

if p14sign_legacy_version "$EXPECTED_VERSION"; then
  p14sign_die "legacy version forbidden: $EXPECTED_VERSION"
fi
if [[ "$EXPECTED_VERSION" == "latest" ]]; then
  p14sign_die "latest forbidden as version identity"
fi
if [[ "$TEST_FIXTURE" == "1" ]]; then
  [[ "$TAG" == "p14sign-test-only-v${EXPECTED_VERSION}" ]] \
    || p14sign_die "test-fixture tag must equal p14sign-test-only-v\${expected-version} (got $TAG)"
else
  [[ "$TAG" == "v${EXPECTED_VERSION}" ]] || p14sign_die "tag must equal v\${expected-version} (got $TAG vs v$EXPECTED_VERSION)"
fi
p14sign_is_commit40 "$EXPECTED_COMMIT" || p14sign_die "expected-commit must be 40 lowercase hex"

# Reject empty / comment-only allowed signers as accidental PASS
if ! LC_ALL=C grep -qE '^[^#[:space:]]' "$ALLOWED_SIGNERS"; then
  p14sign_die "allowed-signers contains no active key entries (NOT_YET_PROVISIONED / empty template)"
fi

# Annotated tag required: git cat-file -t refs/tags/TAG == tag
OBJ_TYPE="$(git -C "$REPO" cat-file -t "refs/tags/${TAG}" 2>/dev/null || true)"
[[ "$OBJ_TYPE" == "tag" ]] || p14sign_die "tag must be annotated (got type='${OBJ_TYPE:-missing}'); lightweight forbidden"

# Enforce principal authorization against the tagger identity (git verify-tag alone
# may accept a key listed under a different principal).
TAGGER_LINE="$(git -C "$REPO" cat-file -p "refs/tags/${TAG}" | awk '/^tagger / {print; exit}')"
TAGGER_EMAIL="$(printf '%s\n' "$TAGGER_LINE" | sed -nE 's/.*<([^>]+)>.*/\1/p')"
[[ -n "$TAGGER_EMAIL" ]] || p14sign_die "unable to parse tagger email from annotated tag"
if ! LC_ALL=C grep -E "^[^#[:space:]].*" "$ALLOWED_SIGNERS" | awk -v email="$TAGGER_EMAIL" '
  {
    split($1, principals, ",")
    for (i in principals) {
      if (principals[i] == email || principals[i] == "*") found=1
    }
  }
  END { exit found ? 0 : 1 }
'; then
  p14sign_die "tagger email not authorized in allowed-signers: $TAGGER_EMAIL"
fi

TARGET="$(git -C "$REPO" rev-list -n 1 "refs/tags/${TAG}")"
TARGET="$(printf '%s' "$TARGET" | tr 'A-F' 'a-f')"
[[ "$TARGET" == "$EXPECTED_COMMIT" ]] || p14sign_die "tag target commit mismatch: $TARGET != $EXPECTED_COMMIT"

git -C "$REPO" \
  -c gpg.format=ssh \
  -c gpg.ssh.allowedSignersFile="$ALLOWED_SIGNERS" \
  verify-tag "$TAG"

echo "TAG_SIGNATURE_VERIFIED=YES" >&2
echo "TAG_TARGET_COMMIT=$TARGET" >&2
echo "TAG_TAGGER_EMAIL=$TAGGER_EMAIL" >&2
