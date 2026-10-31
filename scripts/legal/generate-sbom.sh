#!/usr/bin/env bash
# Generate CycloneDX SBOM for the exyonq release binary (cli/exyonq).
# Cap062 / publication-privacy: strip absolute path+file:// refs (Lexar, /root, …).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUTPUT="${1:-$ROOT/sbom.cdx.json}"
OUTPUT="$(cd "$(dirname "$OUTPUT")" && pwd)/$(basename "$OUTPUT")"
MANIFEST="$ROOT/cli/exyonq/Cargo.toml"
STAGING="$ROOT/cli/exyonq/sbom.cdx.json"

if ! command -v cargo-cyclonedx >/dev/null 2>&1; then
  echo "cargo-cyclonedx not found; install with: cargo install cargo-cyclonedx --locked" >&2
  exit 1
fi

cd "$ROOT"
rm -f "$STAGING"
cargo cyclonedx \
  --manifest-path "$MANIFEST" \
  --format json \
  --override-filename sbom.cdx \
  --spec-version 1.5 \
  --no-build-deps

if [[ ! -f "$STAGING" ]]; then
  echo "expected SBOM at $STAGING" >&2
  exit 1
fi

mv "$STAGING" "$OUTPUT"

# cargo-cyclonedx may leave transient *.cdx.json in workspace crate dirs; keep root artifact only.
while IFS= read -r -d '' stray; do
  rm -f "$stray"
done < <(find "$ROOT" -name '*.cdx.json' -not -path '*/target/*' ! -path "$OUTPUT" -print0)

# Rewrite absolute path+file:// URIs to repo-relative path+file://./… so public
# packages never embed private host paths (/Volumes/Lexar, /root/exyonq-*, …).
python3 - "$OUTPUT" "$ROOT" <<'PY'
import json, pathlib, sys

out = pathlib.Path(sys.argv[1])
root = pathlib.Path(sys.argv[2]).resolve()
root_uri = root.as_uri()  # file:///...
abs_prefix = "path+file://" + root_uri[len("file://") :]
prefixes = {
    abs_prefix,
    "path+file://" + str(root),
}


def sanitize(obj):
    if isinstance(obj, dict):
        return {k: sanitize(v) for k, v in obj.items()}
    if isinstance(obj, list):
        return [sanitize(v) for v in obj]
    if isinstance(obj, str):
        s = obj
        for p in prefixes:
            if s.startswith(p):
                rest = s[len(p) :]
                if rest.startswith("/"):
                    rest = rest[1:]
                return "path+file://./" + rest
        # Deny-all: any absolute path+file:/// must be rewritten or rejected.
        if s.startswith("path+file:///"):
            for marker in (
                "/exyonq-laboratorio/",
                "/exyonq-p14v041/",
                "/exyonq-github/",
                "/exyonq-phase1-",
            ):
                if marker in s:
                    return "path+file://./" + s.split(marker, 1)[1]
            raise SystemExit(f"unsanitized absolute path+file URI in SBOM: {s[:200]}")
        # Also reject file:// absolute download URLs that are not relative.
        if "download_url=file:///" in s:
            raise SystemExit(f"absolute file:// download_url in SBOM: {s[:200]}")
        return s
    return obj


data = json.loads(out.read_text())
data = sanitize(data)
out.write_text(json.dumps(data, indent=2) + "\n")
text = out.read_text()
# Fail-closed residual absolute markers.
import re
if re.search(r"path\+file:///|download_url=file:///", text):
    raise SystemExit("SBOM still contains absolute path+file:// or file:/// URIs")
for bad in ("/Volumes/Lexar", "/Users/", "/root/", "/home/"):
    # Allow only inside rewritten relative refs that no longer contain these.
    if bad in text:
        raise SystemExit(f"SBOM still contains private path marker {bad!r}")
print(f"sanitized absolute path refs in {out}")
PY

echo "wrote $OUTPUT"
