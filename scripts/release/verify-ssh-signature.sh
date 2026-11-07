#!/usr/bin/env bash
# Verify an SSH signature over the release checksum root (artifact authenticity).
set -euo pipefail
LC_ALL=C
export LC_ALL

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/release/lib/p14sign-common.sh
source "$SCRIPT_DIR/lib/p14sign-common.sh"

usage() {
  cat <<'EOF'
Usage:
  verify-ssh-signature.sh \
    --checksums SHA256SUMS.txt \
    --allowed-signers security/signing/exyonq-release-artifact-signers \
    --signature SHA256SUMS.txt.sig \
    [--principal exyonq-release] \
    [--namespace file]

Requirements:
  - ssh-keygen with -Y verify support.
  - The allowed-signers file is public trust material (never a private key).
  - The signature is checked over the exact bytes of SHA256SUMS.txt.

Prints SSH_SIGNATURE_VERIFIED=YES only on success.
Does NOT verify artifact hashes; use verify-checksums.sh for that.
EOF
}

CHECKSUMS=""
ALLOWED_SIGNERS=""
SIGNATURE=""
PRINCIPAL="exyonq-release"
NAMESPACE="file"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --checksums) p14sign_require_arg --checksums "${2:-}"; CHECKSUMS="$2"; shift 2 ;;
    --allowed-signers) p14sign_require_arg --allowed-signers "${2:-}"; ALLOWED_SIGNERS="$2"; shift 2 ;;
    --signature) p14sign_require_arg --signature "${2:-}"; SIGNATURE="$2"; shift 2 ;;
    --principal) p14sign_require_arg --principal "${2:-}"; PRINCIPAL="$2"; shift 2 ;;
    --namespace) p14sign_require_arg --namespace "${2:-}"; NAMESPACE="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) p14sign_die "unknown argument: $1" ;;
  esac
done

p14sign_require_arg --checksums "$CHECKSUMS"
p14sign_require_arg --allowed-signers "$ALLOWED_SIGNERS"
p14sign_require_arg --signature "$SIGNATURE"

[[ -f "$CHECKSUMS" ]] || p14sign_die "checksums not found: $CHECKSUMS"
[[ -f "$ALLOWED_SIGNERS" ]] || p14sign_die "allowed-signers not found: $ALLOWED_SIGNERS"
[[ -f "$SIGNATURE" ]] || p14sign_die "signature not found: $SIGNATURE"
p14sign_reject_symlink "$CHECKSUMS"
p14sign_reject_symlink "$ALLOWED_SIGNERS"
p14sign_reject_symlink "$SIGNATURE"
p14sign_reject_private_key_arg --allowed-signers "$ALLOWED_SIGNERS"

command -v ssh-keygen >/dev/null 2>&1 || p14sign_die "ssh-keygen is required"

ssh-keygen -Y verify \
  -f "$ALLOWED_SIGNERS" \
  -I "$PRINCIPAL" \
  -n "$NAMESPACE" \
  -s "$SIGNATURE" \
  <"$CHECKSUMS"

echo "SSH_SIGNATURE_VERIFIED=YES"
echo "SSH_SIGNATURE_NAMESPACE=$NAMESPACE"
echo "SSH_SIGNATURE_PRINCIPAL=$PRINCIPAL"
echo "SSH_SIGNATURE_OBJECT=$CHECKSUMS"
