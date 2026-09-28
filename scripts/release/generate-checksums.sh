#!/usr/bin/env bash
# Generate deterministic SHA256SUMS.txt from a release-manifest + artifacts (no signing).
set -euo pipefail
LC_ALL=C
export LC_ALL

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/release/lib/p14sign-common.sh
source "$SCRIPT_DIR/lib/p14sign-common.sh"

usage() {
  cat <<'EOF'
Usage:
  generate-checksums.sh \
    --bundle-root DIR \
    [--manifest PATH] \
    [--output PATH] \
    [--force]

Expects:
  DIR/release-manifest.json (or --manifest)
  DIR/artifacts/*

Writes DIR/SHA256SUMS.txt (or --output) including release-manifest.json and all
declared artifacts. Excludes signature/bundle files.
EOF
}

BUNDLE_ROOT=""
MANIFEST=""
OUTPUT=""
FORCE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle-root) p14sign_require_arg --bundle-root "${2:-}"; BUNDLE_ROOT="$2"; shift 2 ;;
    --manifest) p14sign_require_arg --manifest "${2:-}"; MANIFEST="$2"; shift 2 ;;
    --output) p14sign_require_arg --output "${2:-}"; OUTPUT="$2"; shift 2 ;;
    --force) FORCE=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) p14sign_die "unknown argument: $1" ;;
  esac
done

p14sign_require_arg --bundle-root "$BUNDLE_ROOT"
[[ -d "$BUNDLE_ROOT" ]] || p14sign_die "bundle-root not a directory"
BUNDLE_ROOT="$(cd "$BUNDLE_ROOT" && pwd)"
p14sign_reject_symlink "$BUNDLE_ROOT"

MANIFEST="${MANIFEST:-$BUNDLE_ROOT/release-manifest.json}"
OUTPUT="${OUTPUT:-$BUNDLE_ROOT/SHA256SUMS.txt}"
[[ -f "$MANIFEST" ]] || p14sign_die "missing manifest: $MANIFEST"
p14sign_reject_symlink "$MANIFEST"

p14sign_require_python3
p14sign_python3 "$SCRIPT_DIR/lib/manifest.py" validate --manifest "$MANIFEST"

ART_DIR="$BUNDLE_ROOT/artifacts"
[[ -d "$ART_DIR" ]] || p14sign_die "missing artifacts directory: $ART_DIR"

# Validate declared artifacts vs disk, then build SUMS via python for determinism
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT

p14sign_python3 - "$MANIFEST" "$BUNDLE_ROOT" "$TMP" <<'PY'
import json, hashlib, os, sys
from pathlib import Path

manifest_path = Path(sys.argv[1])
root = Path(sys.argv[2])
out = Path(sys.argv[3])
obj = json.loads(manifest_path.read_text(encoding="utf-8"))

def sha256(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()

# Relativize manifest path inside bundle
try:
    man_rel = str(manifest_path.resolve().relative_to(root.resolve()))
except ValueError:
    print("ERROR: manifest must live under bundle-root", file=sys.stderr)
    raise SystemExit(1)
if man_rel != "release-manifest.json":
    # allow only canonical name at root
    print(f"ERROR: manifest must be release-manifest.json at bundle root (got {man_rel})", file=sys.stderr)
    raise SystemExit(1)

entries = []
# manifest first in list then sort all together lexicographically by path
man_hash = sha256(manifest_path)
entries.append((man_rel, man_hash))

declared = set()
for a in obj["artifacts"]:
    rel = a["path"]
    if rel in declared:
        print(f"ERROR: duplicate path {rel}", file=sys.stderr)
        raise SystemExit(1)
    declared.add(rel)
    p = root / rel
    if p.is_symlink():
        print(f"ERROR: symlink forbidden: {rel}", file=sys.stderr)
        raise SystemExit(1)
    if not p.is_file():
        print(f"ERROR: missing artifact: {rel}", file=sys.stderr)
        raise SystemExit(1)
    digest = sha256(p)
    size = p.stat().st_size
    if digest != a["sha256"]:
        print(f"ERROR: sha256 mismatch for {rel}", file=sys.stderr)
        raise SystemExit(1)
    if size != int(a["size_bytes"]):
        print(f"ERROR: size mismatch for {rel}", file=sys.stderr)
        raise SystemExit(1)
    entries.append((rel, digest))

# Reject undeclared files in artifacts/
art_root = root / "artifacts"
for p in sorted(art_root.iterdir(), key=lambda x: x.name):
    if p.name.startswith("."):
        continue
    if p.is_symlink():
        print(f"ERROR: symlink forbidden: artifacts/{p.name}", file=sys.stderr)
        raise SystemExit(1)
    if not p.is_file():
        continue
    rel = f"artifacts/{p.name}"
    if rel not in declared:
        print(f"ERROR: undeclared artifact present: {rel}", file=sys.stderr)
        raise SystemExit(1)

entries.sort(key=lambda t: t[0])
lines = [f"{h}  {path}" for path, h in entries]
out.write_text("\n".join(lines) + "\n", encoding="utf-8")
PY

if [[ -e "$OUTPUT" && "$FORCE" != "1" ]]; then
  p14sign_die "refusing to overwrite $OUTPUT (use --force)"
fi
mv -f "$TMP" "$OUTPUT"
trap - EXIT
echo "wrote $OUTPUT" >&2
