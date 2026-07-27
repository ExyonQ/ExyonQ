#!/usr/bin/env bash
# Verify Cosign key-managed signature over SHA256SUMS.txt (precondition-aware).
set -euo pipefail
LC_ALL=C
export LC_ALL

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/release/lib/p14sign-common.sh
source "$SCRIPT_DIR/lib/p14sign-common.sh"

usage() {
  cat <<'EOF'
Usage:
  verify-cosign-blob.sh \
    --checksums SHA256SUMS.txt \
    --key exyonq-cosign.pub \
    [--bundle SHA256SUMS.txt.sigstore.json | SHA256SUMS.txt.bundle] \
    [--signature SHA256SUMS.txt.sig]

Requires Cosign. Does not use system default keys.
Rejects private-key material passed as --key.
EOF
}

CHECKSUMS=""
KEY=""
BUNDLE=""
SIGNATURE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --checksums) p14sign_require_arg --checksums "${2:-}"; CHECKSUMS="$2"; shift 2 ;;
    --key) p14sign_require_arg --key "${2:-}"; KEY="$2"; shift 2 ;;
    --bundle) p14sign_require_arg --bundle "${2:-}"; BUNDLE="$2"; shift 2 ;;
    --signature) p14sign_require_arg --signature "${2:-}"; SIGNATURE="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) p14sign_die "unknown argument: $1" ;;
  esac
done

p14sign_require_arg --checksums "$CHECKSUMS"
p14sign_require_arg --key "$KEY"
[[ -f "$CHECKSUMS" ]] || p14sign_die "checksums not found: $CHECKSUMS"
p14sign_reject_symlink "$CHECKSUMS"
p14sign_reject_private_key_arg --key "$KEY"
[[ -f "$KEY" ]] || p14sign_die "public key not found: $KEY"

COSIGN="$(p14sign_require_cosign)"

# Soft pin: Cosign 2.x expected (ADR). Exact pin enforced when provisioning CI.
_cosign_v="$("$COSIGN" version 2>/dev/null | awk '/^GitVersion:/ {print $2; exit}')"
if [[ -n "$_cosign_v" && "$_cosign_v" != v2.* ]]; then
  echo "WARNING: Cosign version may be outside designed 2.x range: $_cosign_v" >&2
fi

if [[ -z "$BUNDLE" && -z "$SIGNATURE" ]]; then
  p14sign_die "provide --bundle and/or --signature"
fi

ARGS=(verify-blob --key "$KEY" --insecure-ignore-tlog=true)
if [[ -n "$BUNDLE" ]]; then
  [[ -f "$BUNDLE" ]] || p14sign_die "bundle not found: $BUNDLE"
  p14sign_reject_symlink "$BUNDLE"
  p14sign_reject_private_key_arg --bundle "$BUNDLE"
  ARGS+=(--bundle "$BUNDLE")
fi
if [[ -n "$SIGNATURE" ]]; then
  [[ -f "$SIGNATURE" ]] || p14sign_die "signature not found: $SIGNATURE"
  p14sign_reject_symlink "$SIGNATURE"
  ARGS+=(--signature "$SIGNATURE")
fi
ARGS+=("$CHECKSUMS")

"$COSIGN" "${ARGS[@]}"
echo "COSIGN_BLOB_VERIFIED=YES" >&2
echo "COSIGN_TLOG_REQUIRED=NO" >&2
echo "COSIGN_OFFLINE_KEY_MANAGED=YES" >&2
