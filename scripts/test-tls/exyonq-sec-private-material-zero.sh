#!/usr/bin/env bash
# EXYONQ-SEC-PRIVATE-MATERIAL-ZERO — fail-closed private-key scanner.
# RULE_ID=EXYONQ-SEC-PRIVATE-MATERIAL-ZERO
# No exceptions for tests/fixtures/bench/soak/examples/docs.
set -euo pipefail
LC_ALL=C
export LC_ALL
set +x

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
# shellcheck source=scripts/release/lib/p14sign-common.sh
source "$REPO_ROOT/scripts/release/lib/p14sign-common.sh"

RULE_ID=EXYONQ-SEC-PRIVATE-MATERIAL-ZERO
FAIL_MODE=FAIL_CLOSED

usage() {
  cat <<'EOF'
Usage:
  exyonq-sec-private-material-zero.sh --root DIR
  exyonq-sec-private-material-zero.sh --selftest

Scans DIR for PEM/OpenSSH private-key material. Exit 0 iff count==0.
Does not print key contents — only offending paths.
EOF
}

ROOT=""
SELFTEST=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --root) p14sign_require_arg --root "${2:-}"; ROOT="$2"; shift 2 ;;
    --selftest) SELFTEST=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) p14sign_die "unknown argument: $1" ;;
  esac
done

run_scan() {
  local root="$1"
  local found=0
  local f
  [[ -d "$root" ]] || p14sign_die "root not a directory: $root"
  while IFS= read -r -d '' f; do
    if p14sign_looks_private_key "$f"; then
      echo "ERROR: ${RULE_ID}: private key material found: $f" >&2
      found=$((found + 1))
    fi
  # Prune build caches and extracted third-party tool sources only.
  # NOT an allowlist for tests/fixtures/bench/soak/examples/docs ExyonQ material.
  done < <(find "$root" \
    \( -path '*/.git/*' \
       -o -path '*/target/*' \
       -o -path '*/node_modules/*' \
       -o -path '*/.exyonq-local/*' \
       -o -path '*/benchmarks/tools/http3-loadgen/src/*' \
       -o -path '*/benchmarks/results/*' \
       -o -path '*/benchmarks/rivals-cache/*' \
    \) -prune \
    -o -type f -print0 2>/dev/null)

  echo "RULE_ID=${RULE_ID}"
  echo "FAIL_MODE=${FAIL_MODE}"
  echo "PRIVATE_KEY_FILES_FOUND=${found}"
  [[ "$found" -eq 0 ]]
}

if [[ "$SELFTEST" == "1" ]]; then
  TMP="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-sec-pmz-selftest-XXXXXX")"
  trap 'rm -rf "$TMP"' EXIT
  # Negative: plant ephemeral private key under temp tree — must FAIL
  openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$TMP/planted.key.pem" \
    -out "$TMP/planted.cert.pem" \
    -days 1 -subj '/CN=localhost' >/dev/null 2>&1
  chmod 0600 "$TMP/planted.key.pem"
  if run_scan "$TMP"; then
    echo "ERROR: selftest expected FAIL on planted private key" >&2
    exit 1
  fi
  rm -f "$TMP/planted.key.pem"
  # Positive: only public cert remains — must PASS
  run_scan "$TMP"
  echo "EXYONQ_SEC_PRIVATE_MATERIAL_ZERO_SELFTEST=PASS"
  exit 0
fi

p14sign_require_arg --root "$ROOT"
run_scan "$ROOT"
echo "EXYONQ_SEC_PRIVATE_MATERIAL_ZERO_GATE=PASS"
