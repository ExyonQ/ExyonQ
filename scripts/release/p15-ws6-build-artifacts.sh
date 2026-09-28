#!/usr/bin/env bash
# P1.5-WS6 — Build release artifacts and stage versioned Linux tarball.
# BIT_FOR_BIT NOT_CLAIMED. Uses cargo build --locked.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/release/lib/ws6-common.sh
source "$ROOT/scripts/release/lib/ws6-common.sh"

usage() {
  cat <<'EOF'
Usage:
  p15-ws6-build-artifacts.sh \
    --workspace PATH \
    --target x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu \
    --out-dir PATH \
    [--profile release]
EOF
}

WORKSPACE=""
TARGET=""
OUT_DIR=""
PROFILE="release"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace) WORKSPACE="$2"; shift 2 ;;
    --target) TARGET="$2"; shift 2 ;;
    --out-dir) OUT_DIR="$2"; shift 2 ;;
    --profile) PROFILE="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$WORKSPACE" && -d "$WORKSPACE" && -n "$TARGET" && -n "$OUT_DIR" ]] || {
  usage >&2
  exit 2
}

WS6_REPO_ROOT="$WORKSPACE"
ARCH="$(ws6_target_to_arch "$TARGET")"
VERSION="$(ws6_version_label "$WORKSPACE")"
BASE_VERSION="$(ws6_read_version "$WORKSPACE")"
TARBALL_NAME="$(ws6_tarball_name "$VERSION" "$ARCH")"
STAGE="$(mktemp -d)"
BUILD_TS="$(ws6_build_timestamp)"
TOOLCHAIN="$(ws6_rust_toolchain "$WORKSPACE")"
HEAD="$(ws6_git_head "$WORKSPACE")"
HEAD12="$(ws6_git_head_short "$WORKSPACE")"
DIRTY="$(ws6_git_dirty "$WORKSPACE")"
LOCK_HASH="$(ws6_lockfile_hash "$WORKSPACE")"

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

cleanup() {
  rm -rf "$STAGE"
}
trap cleanup EXIT INT TERM

cd "$WORKSPACE"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"
# shellcheck disable=SC1091
source ~/.cargo/env 2>/dev/null || true

FEATURES=()
while IFS= read -r line; do
  [[ -n "$line" ]] && FEATURES+=("$line")
done < <(ws6_cargo_build_features || true)

# P14V042: inject the exact source SHA into CLI build scripts (no in-container git).
# Official packaging builds fail closed if revision is missing or not a 40-hex SHA.
export EXYONQ_SOURCE_REVISION="$HEAD"
export EXYONQ_OFFICIAL_RELEASE="${EXYONQ_OFFICIAL_RELEASE:-1}"
if [[ ! "$EXYONQ_SOURCE_REVISION" =~ ^[0-9a-f]{40}$ ]]; then
  echo "ERROR: EXYONQ_SOURCE_REVISION must be 40 lowercase hex (got '${EXYONQ_SOURCE_REVISION}')" >&2
  exit 1
fi

ws6_log "building exyonq + exyonqctl target=$TARGET profile=$PROFILE locked=yes source_revision=$EXYONQ_SOURCE_REVISION official=$EXYONQ_OFFICIAL_RELEASE"
cargo build -p exyonq -p exyonqctl --"$PROFILE" --locked --target "$TARGET" "${FEATURES[@]}"

BIN_ROOT="$WORKSPACE/target/$TARGET/$PROFILE"
EXYONQ_BIN="$BIN_ROOT/exyonq"
EXYONQCTL_BIN="$BIN_ROOT/exyonqctl"
[[ -x "$EXYONQ_BIN" && -x "$EXYONQCTL_BIN" ]] || {
  echo "ERROR: expected binaries under $BIN_ROOT" >&2
  exit 1
}

install -d "$STAGE/usr/bin" \
  "$STAGE/etc/exyonq" \
  "$STAGE/usr/lib/systemd/system" \
  "$STAGE/usr/lib/tmpfiles.d" \
  "$STAGE/etc/logrotate.d" \
  "$STAGE/usr/share/doc/exyonq" \
  "$STAGE/usr/share/licenses/exyonq"

install -m 0755 "$EXYONQ_BIN" "$STAGE/usr/bin/exyonq"
install -m 0755 "$EXYONQCTL_BIN" "$STAGE/usr/bin/exyonqctl"
install -m 0644 "$WORKSPACE/packaging/config/config.toml.example" \
  "$STAGE/etc/exyonq/config.toml.example"
install -m 0644 "$WORKSPACE/packaging/systemd/exyonq.service" \
  "$STAGE/usr/lib/systemd/system/exyonq.service"
if [[ -f "$WORKSPACE/packaging/tmpfiles.d/exyonq.conf" ]]; then
  install -m 0644 "$WORKSPACE/packaging/tmpfiles.d/exyonq.conf" \
    "$STAGE/usr/lib/tmpfiles.d/exyonq.conf"
fi
if [[ -f "$WORKSPACE/packaging/logrotate.d/exyonq" ]]; then
  install -m 0644 "$WORKSPACE/packaging/logrotate.d/exyonq" \
    "$STAGE/etc/logrotate.d/exyonq"
fi

if [[ -f "$WORKSPACE/packaging/README.md" ]]; then
  install -m 0644 "$WORKSPACE/packaging/README.md" "$STAGE/usr/share/doc/exyonq/README.md"
else
  printf 'ExyonQ packaging notes — see packaging/README.md in source tree.\n' \
    >"$STAGE/usr/share/doc/exyonq/README.md"
fi

for f in LICENSE NOTICE; do
  if [[ -f "$WORKSPACE/$f" ]]; then
    install -m 0644 "$WORKSPACE/$f" "$STAGE/usr/share/licenses/exyonq/$f"
  fi
done

EXYONQ_SHA="$(ws6_sha256_file "$STAGE/usr/bin/exyonq")"
EXYONQCTL_SHA="$(ws6_sha256_file "$STAGE/usr/bin/exyonqctl")"
JEMALLOC_ENABLED="false"
[[ "${EXYONQ_WS6_JEMALLOC:-0}" == "1" ]] && JEMALLOC_ENABLED="true"

MANIFEST_JSON="$STAGE/build-manifest.json"
MANIFEST_TXT="$STAGE/build-manifest.txt"
TARBALL_PATH="$OUT_DIR/$TARBALL_NAME"

python3 - "$MANIFEST_JSON" <<PY
import json, os
obj = {
  "schema_version": "1.0",
  "product": "exyonq",
  "version": "$BASE_VERSION",
  "version_label": "$VERSION",
  "git": {
    "commit": "$HEAD",
    "commit_short": "$HEAD12",
    "dirty": ("$DIRTY" == "true")
  },
  "build": {
    "profile": "$PROFILE",
    "target": "$TARGET",
    "arch_label": "$ARCH",
    "cargo_locked": True,
    "rust_channel": "$TOOLCHAIN",
    "build_timestamp": "$BUILD_TS",
    "cargo_lock_sha256": "$LOCK_HASH",
    "jemalloc": ("$JEMALLOC_ENABLED" == "true"),
    "harness": "p15-ws6-build-artifacts.sh"
  },
  "binaries": [
    {"name": "exyonq", "path": "usr/bin/exyonq", "sha256": "$EXYONQ_SHA"},
    {"name": "exyonqctl", "path": "usr/bin/exyonqctl", "sha256": "$EXYONQCTL_SHA"}
  ],
  "artifacts": [],
  "integrity": {
    "sha256sums_file": "SHA256SUMS.txt",
    "bit_for_bit_claim": "${WS6_BIT_FOR_BIT_CLAIM}"
  },
  "reproducibility": {
    "classification": "NOT_CLAIMED",
    "note": "BIT_FOR_BIT NOT_CLAIMED; use p15-ws6-reproducibility-check.sh"
  },
  "provenance": {
    "source_head": "$HEAD",
    "source_tree_status": "$DIRTY",
    "rustc_version": os.popen("rustc --version 2>/dev/null").read().strip(),
    "cargo_version": os.popen("cargo --version 2>/dev/null").read().strip(),
    "builder_uname": os.popen("uname -srm 2>/dev/null").read().strip()
  }
}
with open("$MANIFEST_JSON", "w", encoding="utf-8") as f:
    json.dump(obj, f, indent=2, sort_keys=True)
    f.write("\n")
PY

cat >"$MANIFEST_TXT" <<EOF
ExyonQ build manifest (human)
version=$BASE_VERSION
version_label=$VERSION
git_commit=$HEAD
arch=$ARCH
target=$TARGET
build_timestamp=$BUILD_TS
cargo_locked=true
rust_channel=$TOOLCHAIN
cargo_lock_sha256=$LOCK_HASH
exyonq_sha256=$EXYONQ_SHA
exyonqctl_sha256=$EXYONQCTL_SHA
bit_for_bit_claim=${WS6_BIT_FOR_BIT_CLAIM}
EOF

tar -C "$STAGE" -czf "$TARBALL_PATH" \
  usr etc build-manifest.json build-manifest.txt

TARBALL_SHA="$(ws6_sha256_file "$TARBALL_PATH")"

# External provenance only — do NOT re-pack the tarball with its own digest
# (that would invalidate the hash recorded inside the archive).
python3 - "$OUT_DIR/build-manifest.json" "$MANIFEST_JSON" "$TARBALL_NAME" "$TARBALL_SHA" <<'PY'
import json, pathlib, sys
out_path, stage_path, name, sha = sys.argv[1:5]
data = json.loads(pathlib.Path(stage_path).read_text())
data["artifacts"] = [{
    "name": name,
    "kind": "tarball",
    "compression": "gzip",
    "sha256": sha,
    "contract_name": True,
    "note": "sha256 is of the published tarball; not embedded inside the archive",
}]
text = json.dumps(data, indent=2, sort_keys=True) + "\n"
pathlib.Path(out_path).write_text(text)
PY
cp "$MANIFEST_TXT" "$OUT_DIR/build-manifest.txt"
printf '%s  %s\n' "$TARBALL_SHA" "$TARBALL_NAME" >"$OUT_DIR/SHA256SUMS.txt"
SUMMARY_JSON="$(python3 - <<PY
import json
print(json.dumps({
  "script": "p15-ws6-build-artifacts.sh",
  "verdict": "PASS",
  "version": "$BASE_VERSION",
  "version_label": "$VERSION",
  "arch": "$ARCH",
  "target": "$TARGET",
  "tarball": "$TARBALL_NAME",
  "tarball_sha256": "$TARBALL_SHA",
  "out_dir": "$OUT_DIR",
  "cargo_locked": True,
  "bit_for_bit_claim": "${WS6_BIT_FOR_BIT_CLAIM}",
  "build_timestamp": "$BUILD_TS",
}, indent=2))
PY
)"

SUMMARY_TXT="VERDICT=PASS
VERSION=$BASE_VERSION
VERSION_LABEL=$VERSION
ARCH=$ARCH
TARGET=$TARGET
TARBALL=$TARBALL_NAME
TARBALL_SHA256=$TARBALL_SHA
OUT_DIR=$OUT_DIR
CARGO_LOCKED=true
BIT_FOR_BIT=${WS6_BIT_FOR_BIT_CLAIM}"

ws6_write_summary_pair "$OUT_DIR" "$SUMMARY_TXT" "$SUMMARY_JSON"
ws6_log "wrote $TARBALL_PATH"
printf '%s\n' "$SUMMARY_TXT"
printf '%s\n' "$SUMMARY_JSON"
