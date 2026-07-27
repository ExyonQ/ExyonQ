#!/usr/bin/env bash
# Verify SHA256SUMS.txt against release-manifest + artifacts (integrity only).
set -euo pipefail
LC_ALL=C
export LC_ALL

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/release/lib/p14sign-common.sh
source "$SCRIPT_DIR/lib/p14sign-common.sh"

usage() {
  cat <<'EOF'
Usage:
  verify-checksums.sh --bundle-root DIR [--manifest PATH] [--checksums PATH]

Exit 0 only if integrity checks pass.
Does NOT verify cryptographic signatures.
EOF
}

BUNDLE_ROOT=""
MANIFEST=""
CHECKSUMS=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle-root) p14sign_require_arg --bundle-root "${2:-}"; BUNDLE_ROOT="$2"; shift 2 ;;
    --manifest) p14sign_require_arg --manifest "${2:-}"; MANIFEST="$2"; shift 2 ;;
    --checksums) p14sign_require_arg --checksums "${2:-}"; CHECKSUMS="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) p14sign_die "unknown argument: $1" ;;
  esac
done

p14sign_require_arg --bundle-root "$BUNDLE_ROOT"
BUNDLE_ROOT="$(cd "$BUNDLE_ROOT" && pwd)"
MANIFEST="${MANIFEST:-$BUNDLE_ROOT/release-manifest.json}"
CHECKSUMS="${CHECKSUMS:-$BUNDLE_ROOT/SHA256SUMS.txt}"
[[ -f "$MANIFEST" ]] || p14sign_die "missing manifest"
[[ -f "$CHECKSUMS" ]] || p14sign_die "missing checksums"
p14sign_reject_symlink "$MANIFEST"
p14sign_reject_symlink "$CHECKSUMS"
p14sign_require_python3
p14sign_python3 "$SCRIPT_DIR/lib/manifest.py" validate --manifest "$MANIFEST"

p14sign_python3 - "$BUNDLE_ROOT" "$MANIFEST" "$CHECKSUMS" <<'PY'
import hashlib, json, re, sys
from pathlib import Path

root = Path(sys.argv[1])
manifest_path = Path(sys.argv[2])
sums_path = Path(sys.argv[3])
obj = json.loads(manifest_path.read_text(encoding="utf-8"))

HEX = re.compile(r"^[0-9a-f]{64}$")
FORBIDDEN_NAMES = {
    "SHA256SUMS.txt",
    "SHA256SUMS.txt.sig",
    "SHA256SUMS.txt.bundle",
    "SHA256SUMS.txt.sigstore.json",
    "exyonq-cosign.pub",
}

def sha256(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()

lines = sums_path.read_text(encoding="utf-8").splitlines()
seen = {}
for i, line in enumerate(lines, 1):
    if not line.strip():
        print(f"ERROR: empty line {i}", file=sys.stderr)
        raise SystemExit(1)
    if "  " not in line:
        print(f"ERROR: malformed line {i} (need two spaces)", file=sys.stderr)
        raise SystemExit(1)
    digest, path = line.split("  ", 1)
    if not HEX.match(digest):
        print(f"ERROR: bad hash on line {i}", file=sys.stderr)
        raise SystemExit(1)
    if path != path.lower() and False:
        pass
    if path.startswith("/") or ".." in path.split("/"):
        print(f"ERROR: illegal path on line {i}: {path}", file=sys.stderr)
        raise SystemExit(1)
    base = Path(path).name
    if base in FORBIDDEN_NAMES or path in FORBIDDEN_NAMES:
        print(f"ERROR: forbidden entry in SUMS: {path}", file=sys.stderr)
        raise SystemExit(1)
    if path.endswith(".sig") or path.endswith(".bundle") or path.endswith(".sigstore.json"):
        print(f"ERROR: signature/bundle must not appear in SUMS: {path}", file=sys.stderr)
        raise SystemExit(1)
    if path in seen:
        print(f"ERROR: duplicate path {path}", file=sys.stderr)
        raise SystemExit(1)
    seen[path] = digest
    p = root / path
    if p.is_symlink():
        print(f"ERROR: symlink: {path}", file=sys.stderr)
        raise SystemExit(1)
    if not p.is_file():
        print(f"ERROR: missing file: {path}", file=sys.stderr)
        raise SystemExit(1)
    actual = sha256(p)
    if actual != digest:
        print(f"ERROR: hash mismatch: {path}", file=sys.stderr)
        raise SystemExit(1)

if "release-manifest.json" not in seen:
    print("ERROR: release-manifest.json must be listed in SHA256SUMS.txt", file=sys.stderr)
    raise SystemExit(1)

declared = {a["path"] for a in obj["artifacts"]}
for rel in declared:
    if rel not in seen:
        print(f"ERROR: declared artifact missing from SUMS: {rel}", file=sys.stderr)
        raise SystemExit(1)
    # size/hash vs manifest
    a = next(x for x in obj["artifacts"] if x["path"] == rel)
    p = root / rel
    if sha256(p) != a["sha256"] or p.stat().st_size != int(a["size_bytes"]):
        print(f"ERROR: manifest metadata mismatch for {rel}", file=sys.stderr)
        raise SystemExit(1)

# SUMS paths besides manifest must equal declared set
sums_arts = {p for p in seen if p != "release-manifest.json"}
if sums_arts != declared:
    print(f"ERROR: SUMS artifact set != manifest set\n sums={sorted(sums_arts)}\n man={sorted(declared)}", file=sys.stderr)
    raise SystemExit(1)

# No undeclared files in artifacts/
art = root / "artifacts"
if art.is_dir():
    for p in art.iterdir():
        if not p.is_file() and not p.is_symlink():
            continue
        rel = f"artifacts/{p.name}"
        if p.is_symlink():
            print(f"ERROR: symlink in artifacts/: {rel}", file=sys.stderr)
            raise SystemExit(1)
        if rel not in declared:
            print(f"ERROR: undeclared file in artifacts/: {rel}", file=sys.stderr)
            raise SystemExit(1)

print("MANIFEST_HASH_MATCH=YES")
print("ARTIFACT_HASH_MATCH=YES")
print("ARTIFACT_SIZE_MATCH=YES")
print("DECLARED_SET_EQUALS_PRESENT_SET=YES")
print("INTEGRITY_VERIFIED=YES")
print("AUTHENTICITY_VERIFIED=NO")
PY
