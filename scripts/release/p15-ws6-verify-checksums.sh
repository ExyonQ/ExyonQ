#!/usr/bin/env bash
# P1.5-WS6 — Verify SHA256SUMS.txt; negative tamper/missing/arch mismatch tests.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/release/lib/ws6-common.sh
source "$ROOT/scripts/release/lib/ws6-common.sh"

usage() {
  cat <<'EOF'
Usage:
  p15-ws6-verify-checksums.sh \
    --artifact-dir PATH \
    [--expected-arch amd64|arm64] \
    [--run-negative-tests]
EOF
}

ARTIFACT_DIR=""
EXPECTED_ARCH=""
RUN_NEGATIVE="false"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --artifact-dir) ARTIFACT_DIR="$2"; shift 2 ;;
    --expected-arch) EXPECTED_ARCH="$2"; shift 2 ;;
    --run-negative-tests) RUN_NEGATIVE="true"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$ARTIFACT_DIR" && -d "$ARTIFACT_DIR" ]] || {
  usage >&2
  exit 2
}

ARTIFACT_DIR="$(cd "$ARTIFACT_DIR" && pwd)"
CHECKSUMS="$ARTIFACT_DIR/SHA256SUMS.txt"
[[ -f "$CHECKSUMS" ]] || {
  echo "ERROR: missing SHA256SUMS.txt in $ARTIFACT_DIR" >&2
  exit 1
}

OVERALL_RC=0
declare -a CHECK_RESULTS=()

record() {
  local id="$1" verdict="$2" note="${3:-}"
  CHECK_RESULTS+=("$id:$verdict:$note")
  echo "[$id] $verdict ${note:+- $note}"
  if [[ "$verdict" == "FAIL" ]]; then
    OVERALL_RC=1
  fi
  return 0
}

# Positive verification
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$ARTIFACT_DIR" && sha256sum -c SHA256SUMS.txt) && \
    record "POSITIVE_VERIFY" "PASS" "sha256sum -c" || \
    record "POSITIVE_VERIFY" "FAIL" "sha256sum -c"
else
  while read -r expected name; do
    [[ -n "$expected" && -n "$name" ]] || continue
    file="$ARTIFACT_DIR/$name"
    if [[ ! -f "$file" ]]; then
      record "POSITIVE_VERIFY" "FAIL" "missing $name"
      continue
    fi
    actual="$(ws6_sha256_file "$file")"
    if [[ "$actual" == "$expected" ]]; then
      record "POSITIVE_VERIFY" "PASS" "$name"
    else
      record "POSITIVE_VERIFY" "FAIL" "hash mismatch $name"
    fi
  done <"$CHECKSUMS"
fi

# Arch name vs host/expected
while read -r _ name; do
  [[ -n "$name" ]] || continue
  tarball_arch=""
  tarball_arch="$(ws6_extract_arch_from_tarball_name "$name" 2>/dev/null || true)"
  if [[ -n "$EXPECTED_ARCH" && -n "$tarball_arch" && "$tarball_arch" != "$EXPECTED_ARCH" ]]; then
    record "ARCH_NAME_MISMATCH" "FAIL" "tarball=$tarball_arch expected=$EXPECTED_ARCH"
  elif [[ -n "$tarball_arch" && "$(uname -s)" == "Linux" ]]; then
    host_arch="$(ws6_host_arch_label)"
    if [[ "$tarball_arch" != "$host_arch" ]]; then
      record "ARCH_HOST_MISMATCH" "WARN" "tarball=$tarball_arch host=$host_arch"
    else
      record "ARCH_HOST_MATCH" "PASS" "tarball=$tarball_arch"
    fi
  fi
done <"$CHECKSUMS"

if [[ "$RUN_NEGATIVE" == "true" ]]; then
  TAMPER_DIR="$(mktemp -d)"
  cp -a "$ARTIFACT_DIR/." "$TAMPER_DIR/"
  tarball="$(find "$TAMPER_DIR" -maxdepth 1 -name 'exyonq-*-linux-*.tar.gz' | head -n1 || true)"
  if [[ -n "$tarball" ]]; then
    printf '\0' >>"$tarball"
    if (cd "$TAMPER_DIR" && sha256sum -c SHA256SUMS.txt >/dev/null 2>&1); then
      record "TAMPER_TEST" "FAIL" "corrupted tarball still verified"
    else
      record "TAMPER_TEST" "PASS" "corrupted tarball rejected"
    fi
  else
    record "TAMPER_TEST" "SKIP" "no tarball to corrupt"
  fi

  MISSING_DIR="$(mktemp -d)"
  cp "$CHECKSUMS" "$MISSING_DIR/"
  if (cd "$MISSING_DIR" && sha256sum -c SHA256SUMS.txt >/dev/null 2>&1); then
    record "MISSING_FILE_TEST" "FAIL" "missing artifact still verified"
  else
    record "MISSING_FILE_TEST" "PASS" "missing artifact rejected"
  fi
  rm -rf "$TAMPER_DIR" "$MISSING_DIR"
fi

VERDICT="PASS"
[[ "$OVERALL_RC" -ne 0 ]] && VERDICT="FAIL"

SUMMARY_JSON="$(python3 - <<PY
import json
checks = []
for item in """${CHECK_RESULTS[*]}""".split():
    if not item:
        continue
    parts = item.split(":", 2)
    if len(parts) == 3:
        checks.append({"id": parts[0], "verdict": parts[1], "note": parts[2]})
print(json.dumps({
  "script": "p15-ws6-verify-checksums.sh",
  "verdict": "$VERDICT",
  "artifact_dir": "$ARTIFACT_DIR",
  "checks": checks,
}, indent=2))
PY
)"

SUMMARY_TXT="VERDICT=$VERDICT
ARTIFACT_DIR=$ARTIFACT_DIR
CHECKS=${CHECK_RESULTS[*]}"

ws6_write_summary_pair "$ARTIFACT_DIR" "$SUMMARY_TXT" "$SUMMARY_JSON"
printf '%s\n' "$SUMMARY_TXT"
printf '%s\n' "$SUMMARY_JSON"
exit "$OVERALL_RC"
