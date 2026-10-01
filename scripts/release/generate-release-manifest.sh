#!/usr/bin/env bash
# Generate release-manifest.json (P14SIGN Phase 2 — no signing).
set -euo pipefail
LC_ALL=C
export LC_ALL

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/release/lib/p14sign-common.sh
source "$SCRIPT_DIR/lib/p14sign-common.sh"

usage() {
  cat <<'EOF'
Usage:
  generate-release-manifest.sh \
    --version VERSION \
    --git-commit SHA40 \
    --artifacts-dir DIR \
    --output PATH \
    [--oci-image repo@sha256:DIGEST]... \
    [--created-at RFC3339] \
    [--test-fixture] \
    [--force]

Without --test-fixture, only versions 0.4.0, 0.4.1, 0.4.2, 0.4.3, or 0.4.4 are accepted.
With --test-fixture, approved fixture versions only
(0.0.0-p14sign-fixture or 0.4.0-test.p14sign5).
Legacy 0.3.x / 0.2.x / 0.1.x are always forbidden.
EOF
}

VERSION=""
GIT_COMMIT=""
ARTIFACTS_DIR=""
OUTPUT=""
CREATED_AT=""
TEST_FIXTURE=0
FORCE=0
OCI_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) p14sign_require_arg --version "${2:-}"; VERSION="$2"; shift 2 ;;
    --git-commit) p14sign_require_arg --git-commit "${2:-}"; GIT_COMMIT="$2"; shift 2 ;;
    --artifacts-dir) p14sign_require_arg --artifacts-dir "${2:-}"; ARTIFACTS_DIR="$2"; shift 2 ;;
    --output) p14sign_require_arg --output "${2:-}"; OUTPUT="$2"; shift 2 ;;
    --created-at) p14sign_require_arg --created-at "${2:-}"; CREATED_AT="$2"; shift 2 ;;
    --oci-image) p14sign_require_arg --oci-image "${2:-}"; OCI_ARGS+=(--oci-image "$2"); shift 2 ;;
    --test-fixture) TEST_FIXTURE=1; shift ;;
    --force) FORCE=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) p14sign_die "unknown argument: $1" ;;
  esac
done

p14sign_require_arg --version "$VERSION"
p14sign_require_arg --git-commit "$GIT_COMMIT"
p14sign_require_arg --artifacts-dir "$ARTIFACTS_DIR"
p14sign_require_arg --output "$OUTPUT"
p14sign_assert_version_policy "$VERSION" "$TEST_FIXTURE"
p14sign_is_commit40 "$GIT_COMMIT" || p14sign_die "git-commit must be 40 lowercase hex"
[[ -d "$ARTIFACTS_DIR" ]] || p14sign_die "artifacts-dir not a directory: $ARTIFACTS_DIR"
p14sign_reject_symlink "$ARTIFACTS_DIR"

p14sign_require_python3
ARGS=(
  "$SCRIPT_DIR/lib/manifest.py" generate
  --version "$VERSION"
  --git-commit "$GIT_COMMIT"
  --artifacts-dir "$ARTIFACTS_DIR"
  --output "$OUTPUT"
)
[[ -n "$CREATED_AT" ]] && ARGS+=(--created-at "$CREATED_AT")
[[ "$TEST_FIXTURE" == "1" ]] && ARGS+=(--test-fixture)
[[ "$FORCE" == "1" ]] && ARGS+=(--force)
ARGS+=("${OCI_ARGS[@]}")
p14sign_python3 "${ARGS[@]}"
