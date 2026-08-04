#!/usr/bin/env bash
# Release / publication private-material gates (EXYONQ-SEC-PRIVATE-MATERIAL-ZERO).
# Run BEFORE final signatures.
set -euo pipefail
LC_ALL=C
export LC_ALL

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCAN="$ROOT/scripts/security/scan-private-material.sh"
MODE="${1:-}"

usage() {
  cat <<'EOF'
Usage:
  private-material-release-gates.sh preflight
  private-material-release-gates.sh archive <tarball|dir>
  private-material-release-gates.sh oci <oci-layout|image-tar>
  private-material-release-gates.sh assets <dir>
  private-material-release-gates.sh final <bundle-root>

Exit != 0 blocks signing / tag / release / GHCR public visibility.
EOF
}

[[ -n "$MODE" ]] || { usage >&2; exit 2; }

case "$MODE" in
  preflight)
    bash "$SCAN" --git-tree --repo "$ROOT"
    bash "$ROOT/scripts/verify-no-private-paths.sh"
    bash "$ROOT/scripts/security/tests/test-private-material-scanner.sh"
    echo "PRIVATE_MATERIAL_TREE_GATE=PASS"
    echo "PRIVATE_MATERIAL_RELEASE_PREFLIGHT=PASS"
    ;;
  archive)
    target="${2:-}"
    [[ -n "$target" ]] || { echo "ERROR: archive path required" >&2; exit 2; }
    if [[ -d "$target" ]]; then
      bash "$SCAN" --tree "$target"
    else
      bash "$SCAN" --archive "$target"
    fi
    echo "PRIVATE_MATERIAL_ARCHIVE_GATE=PASS"
    ;;
  oci)
    target="${2:-}"
    [[ -n "$target" ]] || { echo "ERROR: oci path required" >&2; exit 2; }
    bash "$SCAN" --oci "$target"
    echo "PRIVATE_MATERIAL_OCI_GATE=PASS"
    ;;
  assets)
    target="${2:-}"
    [[ -n "$target" && -d "$target" ]] || { echo "ERROR: assets dir required" >&2; exit 2; }
    bash "$SCAN" --assets "$target"
    echo "PUBLIC_ASSET_ALLOWLIST_GATE=PASS"
    ;;
  final)
    target="${2:-}"
    [[ -n "$target" && -d "$target" ]] || { echo "ERROR: bundle root required" >&2; exit 2; }
    bash "$SCAN" --tree "$target"
    bash "$ROOT/scripts/verify-no-private-paths.sh"
    echo "LOCAL_PATH_LEAK_GATE=PASS"
    echo "FINAL_RELEASE_LEAK_GATE=PASS"
    ;;
  -h|--help) usage; exit 0 ;;
  *) echo "ERROR: unknown mode: $MODE" >&2; usage >&2; exit 2 ;;
esac
