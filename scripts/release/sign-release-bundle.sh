#!/usr/bin/env bash
# Sign SHA256SUMS.txt with the owner-held ED25519 release key.
# The private key is read only by ssh-keygen; this script never prints it.
set -euo pipefail
LC_ALL=C
export LC_ALL
umask 077

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/release/lib/p14sign-common.sh
source "$SCRIPT_DIR/lib/p14sign-common.sh"

usage() {
  cat <<'EOF'
Usage:
  sign-release-bundle.sh \
    --bundle-root DIR \
    --signing-key PATH \
    --allowed-signers PATH \
    [--expected-fingerprint SHA256:...] \
    [--principal exyonq-release] \
    [--namespace file] \
    [--force]

The key must be owner-only (0400 or 0600), outside the bundle, and its public
fingerprint must match --expected-fingerprint when supplied. The generated
SHA256SUMS.txt.sig is verified immediately against allowed-signers.
EOF
}

BUNDLE_ROOT=""
SIGNING_KEY=""
ALLOWED_SIGNERS=""
EXPECTED_FP=""
PRINCIPAL="exyonq-release"
NAMESPACE="file"
FORCE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle-root) p14sign_require_arg --bundle-root "${2:-}"; BUNDLE_ROOT="$2"; shift 2 ;;
    --signing-key) p14sign_require_arg --signing-key "${2:-}"; SIGNING_KEY="$2"; shift 2 ;;
    --allowed-signers) p14sign_require_arg --allowed-signers "${2:-}"; ALLOWED_SIGNERS="$2"; shift 2 ;;
    --expected-fingerprint) p14sign_require_arg --expected-fingerprint "${2:-}"; EXPECTED_FP="$2"; shift 2 ;;
    --principal) p14sign_require_arg --principal "${2:-}"; PRINCIPAL="$2"; shift 2 ;;
    --namespace) p14sign_require_arg --namespace "${2:-}"; NAMESPACE="$2"; shift 2 ;;
    --force) FORCE=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) p14sign_die "unknown argument: $1" ;;
  esac
done

p14sign_require_arg --bundle-root "$BUNDLE_ROOT"
p14sign_require_arg --signing-key "$SIGNING_KEY"
p14sign_require_arg --allowed-signers "$ALLOWED_SIGNERS"
[[ -d "$BUNDLE_ROOT" ]] || p14sign_die "bundle root not a directory: $BUNDLE_ROOT"
BUNDLE_ROOT="$(cd "$BUNDLE_ROOT" && pwd)"
p14sign_reject_symlink "$BUNDLE_ROOT"
[[ -f "$BUNDLE_ROOT/release-manifest.json" ]] || p14sign_die "missing release-manifest.json"
[[ -f "$BUNDLE_ROOT/SHA256SUMS.txt" ]] || p14sign_die "missing SHA256SUMS.txt"
[[ -f "$SIGNING_KEY" ]] || p14sign_die "signing key not found: $SIGNING_KEY"
[[ -f "$ALLOWED_SIGNERS" ]] || p14sign_die "allowed-signers not found: $ALLOWED_SIGNERS"
p14sign_reject_symlink "$SIGNING_KEY"
p14sign_reject_symlink "$ALLOWED_SIGNERS"
p14sign_reject_private_key_arg --allowed-signers "$ALLOWED_SIGNERS"
command -v ssh-keygen >/dev/null 2>&1 || p14sign_die "ssh-keygen is required"

KEY_ABS="$(cd "$(dirname "$SIGNING_KEY")" && pwd)/$(basename "$SIGNING_KEY")"
case "$KEY_ABS" in
  "$BUNDLE_ROOT"/*) p14sign_die "signing key must live outside the release bundle" ;;
esac

KEY_MODE="$(stat -c '%a' "$SIGNING_KEY" 2>/dev/null || stat -f '%Lp' "$SIGNING_KEY")"
case "$KEY_MODE" in
  400|600) ;;
  *) p14sign_die "signing key must be owner-only (0400 or 0600; got $KEY_MODE)" ;;
esac

ACTUAL_FP="$(ssh-keygen -lf "$SIGNING_KEY" | awk '{print $2}')"
[[ -n "$ACTUAL_FP" ]] || p14sign_die "could not read signing-key fingerprint"
if [[ -n "$EXPECTED_FP" && "$ACTUAL_FP" != "$EXPECTED_FP" ]]; then
  p14sign_die "signing-key fingerprint mismatch (expected $EXPECTED_FP, got $ACTUAL_FP)"
fi

SIGNATURE="$BUNDLE_ROOT/SHA256SUMS.txt.sig"
if [[ -e "$SIGNATURE" ]]; then
  [[ "$FORCE" == "1" ]] || p14sign_die "signature already exists (use --force): $SIGNATURE"
  rm -f "$SIGNATURE"
fi

echo "SIGNING_KEY_FINGERPRINT=$ACTUAL_FP"
echo "SIGNING_KEY_ID=EXYONQ_RELEASE_TAG_SSH_V2"
ssh-keygen -Y sign -f "$SIGNING_KEY" -n "$NAMESPACE" "$BUNDLE_ROOT/SHA256SUMS.txt" >&2
[[ -s "$SIGNATURE" ]] || p14sign_die "signature was not created"

bash "$SCRIPT_DIR/verify-ssh-signature.sh" \
  --checksums "$BUNDLE_ROOT/SHA256SUMS.txt" \
  --allowed-signers "$ALLOWED_SIGNERS" \
  --signature "$SIGNATURE" \
  --principal "$PRINCIPAL" \
  --namespace "$NAMESPACE"

echo "SIGNATURE_FILE=$SIGNATURE"
echo "SIGNING_RESULT=PASS"
