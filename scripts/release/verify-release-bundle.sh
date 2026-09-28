#!/usr/bin/env bash
# Entrypoint: verify an ExyonQ release bundle (P14SIGN Phase 2).
set -euo pipefail
LC_ALL=C
export LC_ALL

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/release/lib/p14sign-common.sh
source "$SCRIPT_DIR/lib/p14sign-common.sh"

usage() {
  cat <<'EOF'
Usage:
  verify-release-bundle.sh --bundle-root DIR [modes] [options]

Modes (pick one; default --integrity-only):
  --integrity-only
  --verify-artifact-signature
  --verify-tag
  --verify-oci
  --all

Common:
  --bundle-root DIR

Artifact signature:
  --cosign-key PATH
  --cosign-bundle PATH
  --cosign-signature PATH

Tag verification:
  --git-repo PATH
  --tag TAG
  --allowed-signers PATH

OCI:
  --oci-image REPO@sha256:DIGEST
  --oci-key PATH   (defaults to --cosign-key)

--integrity-only never claims authenticity.
--all fails if any required tool/material is missing.
EOF
}

BUNDLE_ROOT=""
MODE="integrity"
COSIGN_KEY=""
COSIGN_BUNDLE=""
COSIGN_SIG=""
GIT_REPO=""
TAG=""
ALLOWED_SIGNERS=""
OCI_IMAGE=""
OCI_KEY=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle-root) p14sign_require_arg --bundle-root "${2:-}"; BUNDLE_ROOT="$2"; shift 2 ;;
    --integrity-only) MODE="integrity"; shift ;;
    --verify-artifact-signature) MODE="artifact"; shift ;;
    --verify-tag) MODE="tag"; shift ;;
    --verify-oci) MODE="oci"; shift ;;
    --all) MODE="all"; shift ;;
    --cosign-key) p14sign_require_arg --cosign-key "${2:-}"; COSIGN_KEY="$2"; shift 2 ;;
    --cosign-bundle) p14sign_require_arg --cosign-bundle "${2:-}"; COSIGN_BUNDLE="$2"; shift 2 ;;
    --cosign-signature) p14sign_require_arg --cosign-signature "${2:-}"; COSIGN_SIG="$2"; shift 2 ;;
    --git-repo) p14sign_require_arg --git-repo "${2:-}"; GIT_REPO="$2"; shift 2 ;;
    --tag) p14sign_require_arg --tag "${2:-}"; TAG="$2"; shift 2 ;;
    --allowed-signers) p14sign_require_arg --allowed-signers "${2:-}"; ALLOWED_SIGNERS="$2"; shift 2 ;;
    --oci-image) p14sign_require_arg --oci-image "${2:-}"; OCI_IMAGE="$2"; shift 2 ;;
    --oci-key) p14sign_require_arg --oci-key "${2:-}"; OCI_KEY="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) p14sign_die "unknown argument: $1" ;;
  esac
done

p14sign_require_arg --bundle-root "$BUNDLE_ROOT"
BUNDLE_ROOT="$(cd "$BUNDLE_ROOT" && pwd)"
p14sign_reject_symlink "$BUNDLE_ROOT"

MANIFEST="$BUNDLE_ROOT/release-manifest.json"
SUMS="$BUNDLE_ROOT/SHA256SUMS.txt"
[[ -f "$MANIFEST" ]] || p14sign_die "missing release-manifest.json"
[[ -f "$SUMS" ]] || p14sign_die "missing SHA256SUMS.txt"
[[ -d "$BUNDLE_ROOT/artifacts" ]] || p14sign_die "missing artifacts/"

# Private-key scan before dirty-tree so key material is classified correctly.
if ! p14sign_scan_path_for_private_keys "$BUNDLE_ROOT"; then
  p14sign_die "private key material inside release bundle (FORBIDDEN)"
fi

p14sign_assert_bundle_tree_clean "$BUNDLE_ROOT"

# Reject unexpected symlinks at bundle root / artifacts
while IFS= read -r -d '' p; do
  p14sign_die "unexpected symlink in bundle: $p"
done < <(find "$BUNDLE_ROOT" \( -type l \) -print0 2>/dev/null)

p14sign_require_python3
p14sign_python3 "$SCRIPT_DIR/lib/manifest.py" validate --manifest "$MANIFEST"

VERSION="$(p14sign_python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$MANIFEST")"
GIT_TAG="$(p14sign_python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["git_tag"])' "$MANIFEST")"
GIT_COMMIT="$(p14sign_python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["git_commit"])' "$MANIFEST")"
if p14sign_legacy_version "$VERSION"; then
  p14sign_die "legacy version in manifest: $VERSION"
fi
if [[ "$VERSION" == "latest" ]]; then
  p14sign_die "latest forbidden as version identity"
fi
[[ "$GIT_TAG" == "v${VERSION}" ]] || p14sign_die "git_tag/version mismatch"

echo "=== 1-5 integrity ===" >&2
bash "$SCRIPT_DIR/verify-checksums.sh" --bundle-root "$BUNDLE_ROOT"

AUTH=NO
if [[ "$MODE" == "integrity" ]]; then
  echo "INTEGRITY_VERIFIED=YES"
  echo "AUTHENTICITY_VERIFIED=NO"
  exit 0
fi

run_artifact() {
  p14sign_require_arg --cosign-key "$COSIGN_KEY"
  local bundle="${COSIGN_BUNDLE:-}"
  if [[ -z "$bundle" ]]; then
    if [[ -f "$BUNDLE_ROOT/SHA256SUMS.txt.sigstore.json" ]]; then
      bundle="$BUNDLE_ROOT/SHA256SUMS.txt.sigstore.json"
    elif [[ -f "$BUNDLE_ROOT/SHA256SUMS.txt.bundle" ]]; then
      bundle="$BUNDLE_ROOT/SHA256SUMS.txt.bundle"
    fi
  fi
  local args=(--checksums "$SUMS" --key "$COSIGN_KEY")
  [[ -n "$bundle" ]] && args+=(--bundle "$bundle")
  [[ -n "$COSIGN_SIG" ]] && args+=(--signature "$COSIGN_SIG")
  [[ -n "$bundle" || -n "$COSIGN_SIG" ]] || p14sign_die "missing Cosign bundle/signature for artifact verify"
  bash "$SCRIPT_DIR/verify-cosign-blob.sh" "${args[@]}"
}

run_tag() {
  p14sign_require_arg --git-repo "$GIT_REPO"
  p14sign_require_arg --tag "${TAG:-$GIT_TAG}"
  p14sign_require_arg --allowed-signers "$ALLOWED_SIGNERS"
  bash "$SCRIPT_DIR/verify-tag-signature.sh" \
    --repo "$GIT_REPO" \
    --tag "${TAG:-$GIT_TAG}" \
    --expected-version "$VERSION" \
    --expected-commit "$GIT_COMMIT" \
    --allowed-signers "$ALLOWED_SIGNERS"
}

run_oci() {
  local key="${OCI_KEY:-$COSIGN_KEY}"
  p14sign_require_arg --oci-image "$OCI_IMAGE"
  p14sign_require_arg --oci-key-or-cosign-key "$key"
  bash "$SCRIPT_DIR/verify-oci-signature.sh" --image "$OCI_IMAGE" --key "$key"
}

case "$MODE" in
  artifact) run_artifact; AUTH=YES ;;
  tag) run_tag; AUTH=YES ;;
  oci) run_oci; AUTH=YES ;;
  all)
    run_artifact
    run_tag
    # OCI optional if no images in manifest and no --oci-image
    if [[ -n "$OCI_IMAGE" ]]; then
      run_oci
    else
      COUNT="$(p14sign_python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))["container_images"]))' "$MANIFEST")"
      if [[ "$COUNT" != "0" ]]; then
        # Require explicit --oci-image for --all when containers declared
        p14sign_die "--all with container_images requires --oci-image matching a declared digest"
      fi
    fi
    AUTH=YES
    ;;
  *) p14sign_die "unknown mode" ;;
esac

echo "INTEGRITY_VERIFIED=YES"
echo "AUTHENTICITY_VERIFIED=$AUTH"
