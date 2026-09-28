#!/usr/bin/env bash
# Verify Cosign signature on an OCI digest reference (no tag-only refs).
set -euo pipefail
LC_ALL=C
export LC_ALL

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/release/lib/p14sign-common.sh
source "$SCRIPT_DIR/lib/p14sign-common.sh"

usage() {
  cat <<'EOF'
Usage:
  verify-oci-signature.sh \
    --image REGISTRY/REPO@sha256:DIGEST \
    --key exyonq-cosign.pub \
    [--dry-run-preconditions]

--dry-run-preconditions validates parsing/guards without calling cosign/registry.
EOF
}

IMAGE=""
KEY=""
DRY=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --image) p14sign_require_arg --image "${2:-}"; IMAGE="$2"; shift 2 ;;
    --key) p14sign_require_arg --key "${2:-}"; KEY="$2"; shift 2 ;;
    --dry-run-preconditions) DRY=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) p14sign_die "unknown argument: $1" ;;
  esac
done

p14sign_require_arg --image "$IMAGE"
p14sign_require_arg --key "$KEY"

case "$IMAGE" in
  *:latest|*:latest@*|*/latest@*) p14sign_die "latest forbidden" ;;
esac
[[ "$IMAGE" == *@sha256:* ]] || p14sign_die "image must be repository@sha256:<digest> (tag-only forbidden)"

# Parse OCI digest ref without mapfile (macOS bash 3.2 compatible)
REPO=""
DIG=""
while IFS= read -r line; do
  if [[ -z "$REPO" ]]; then
    REPO="$line"
  else
    DIG="$line"
  fi
done < <(p14sign_parse_oci_digest_ref "$IMAGE")
[[ -n "$REPO" && -n "$DIG" ]] || p14sign_die "failed to parse OCI image ref"
REF="${REPO}@sha256:${DIG}"

if [[ "$DRY" == "1" ]]; then
  if [[ -e "$KEY" ]]; then
    p14sign_reject_private_key_arg --key "$KEY"
  fi
  echo "OCI_PRECONDITIONS_OK=YES"
  echo "OCI_REF=$REF"
  exit 0
fi

p14sign_reject_private_key_arg --key "$KEY"
[[ -f "$KEY" ]] || p14sign_die "public key not found: $KEY"

COSIGN="$(p14sign_require_cosign)"

"$COSIGN" verify --key "$KEY" "$REF"
echo "OCI_SIGNATURE_VERIFIED=YES" >&2
