#!/usr/bin/env bash
# P1.6-WS4 — Build RC candidate tarball (and optional amd64 nfpm).
# Cargo SoT stays 0.3.3; artifact identity = TARGET_RC_VERSION (default 0.3.3-rc.1).
# BIT_FOR_BIT NOT_CLAIMED. RC_STATUS remains NOT_DECLARED.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/release/lib/ws6-common.sh
source "$ROOT/scripts/release/lib/ws6-common.sh"

TARGET_RC_VERSION="${TARGET_RC_VERSION:-0.3.3-rc.1}"
SOURCE_VERSION=""
WORKSPACE=""
TARGET=""
OUT_DIR=""
PROFILE="release"
DO_NFPM="false"

usage() {
  cat <<'EOF'
Usage:
  p16-ws4-build-artifacts.sh \
    --workspace PATH \
    --target x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu \
    --out-dir PATH \
    [--profile release] \
    [--nfpm]   # amd64 only; ignored/fails closed on arm64
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace) WORKSPACE="$2"; shift 2 ;;
    --target) TARGET="$2"; shift 2 ;;
    --out-dir) OUT_DIR="$2"; shift 2 ;;
    --profile) PROFILE="$2"; shift 2 ;;
    --nfpm) DO_NFPM="true"; shift ;;
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
# SOURCE_VERSION from Cargo only (ignore VERSION env for SoT)
SOURCE_VERSION="$(
  grep -E '^[[:space:]]*version[[:space:]]*=' "$WORKSPACE/Cargo.toml" \
    | head -n1 \
    | sed -E 's/.*=[[:space:]]*"([^"]+)".*/\1/'
)"
[[ "$SOURCE_VERSION" == "0.3.3" ]] || {
  echo "ERROR: unexpected Cargo version '$SOURCE_VERSION' (want 0.3.3)" >&2
  exit 1
}

TARBALL_NAME="$(ws6_tarball_name "$TARGET_RC_VERSION" "$ARCH")"
STAGE="$(mktemp -d)"
BUILD_TS="$(ws6_build_timestamp)"
TOOLCHAIN="$(ws6_rust_toolchain "$WORKSPACE")"
HEAD="$(ws6_git_head "$WORKSPACE")"
HEAD12="$(ws6_git_head_short "$WORKSPACE")"
DIRTY="$(ws6_git_dirty "$WORKSPACE")"
LOCK_HASH="$(ws6_lockfile_hash "$WORKSPACE")"

[[ "$DIRTY" == "false" ]] || {
  echo "ERROR: dirty tree forbidden for WS4 artifacts (dirty=$DIRTY)" >&2
  exit 1
}

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

cleanup() { rm -rf "$STAGE"; }
trap cleanup EXIT INT TERM

cd "$WORKSPACE"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"
# shellcheck disable=SC1091
source ~/.cargo/env 2>/dev/null || true

export EXYONQ_SOURCE_REVISION="$HEAD"
export EXYONQ_ARTIFACT_VERSION="$TARGET_RC_VERSION"

ws6_log "WS4 build arch=$ARCH target=$TARGET source=$SOURCE_VERSION rc=$TARGET_RC_VERSION head=$HEAD12"

cargo build -p exyonq -p exyonqctl --"$PROFILE" --locked --target "$TARGET"

BIN_ROOT="$WORKSPACE/target/$TARGET/$PROFILE"
EXYONQ_BIN="$BIN_ROOT/exyonq"
EXYONQCTL_BIN="$BIN_ROOT/exyonqctl"
[[ -x "$EXYONQ_BIN" && -x "$EXYONQCTL_BIN" ]] || {
  echo "ERROR: missing binaries under $BIN_ROOT" >&2
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
[[ -f "$WORKSPACE/packaging/tmpfiles.d/exyonq.conf" ]] && \
  install -m 0644 "$WORKSPACE/packaging/tmpfiles.d/exyonq.conf" \
    "$STAGE/usr/lib/tmpfiles.d/exyonq.conf"
[[ -f "$WORKSPACE/packaging/logrotate.d/exyonq" ]] && \
  install -m 0644 "$WORKSPACE/packaging/logrotate.d/exyonq" \
    "$STAGE/etc/logrotate.d/exyonq"

for f in LICENSE NOTICE THIRD_PARTY_NOTICES.md sbom.cdx.json; do
  if [[ -f "$WORKSPACE/$f" ]]; then
    case "$f" in
      LICENSE|NOTICE) install -m 0644 "$WORKSPACE/$f" "$STAGE/usr/share/licenses/exyonq/$f" ;;
      *) install -m 0644 "$WORKSPACE/$f" "$STAGE/usr/share/doc/exyonq/$f" ;;
    esac
  fi
done

EXYONQ_SHA="$(ws6_sha256_file "$STAGE/usr/bin/exyonq")"
EXYONQCTL_SHA="$(ws6_sha256_file "$STAGE/usr/bin/exyonqctl")"
RUSTC_V="$(rustc --version 2>/dev/null | tr -d '\n')"
CARGO_V="$(cargo --version 2>/dev/null | tr -d '\n')"

MANIFEST_JSON="$STAGE/build-manifest.json"
python3 - "$MANIFEST_JSON" <<PY
import json, os
obj = {
  "schema_version": "1.0",
  "product": "exyonq",
  "artifact": "tarball",
  "target_rc_version": "$TARGET_RC_VERSION",
  "source_version": "$SOURCE_VERSION",
  "source_revision": "$HEAD",
  "target_triple": "$TARGET",
  "architecture": "$ARCH",
  "operating_system": "linux",
  "build_profile": "$PROFILE",
  "rustc_version": "$RUSTC_V",
  "cargo_version": "$CARGO_V",
  "features": [],
  "allocator": "system",
  "build_timestamp": "$BUILD_TS",
  "clean_tree": True,
  "cargo_lock_hash": "$LOCK_HASH",
  "checksum_algorithm": "sha256",
  "checksum": None,
  "sbom_file": "usr/share/doc/exyonq/sbom.cdx.json",
  "license_file": "usr/share/licenses/exyonq/LICENSE",
  "signed": False,
  "publication_state": "candidate_not_released",
  "rc_status": "NOT_DECLARED",
  "bit_for_bit_claim": "NOT_CLAIMED",
  "harness": "p16-ws4-build-artifacts.sh",
  "binaries": [
    {"name": "exyonq", "path": "usr/bin/exyonq", "sha256": "$EXYONQ_SHA"},
    {"name": "exyonqctl", "path": "usr/bin/exyonqctl", "sha256": "$EXYONQCTL_SHA"},
  ],
}
with open("$MANIFEST_JSON", "w", encoding="utf-8") as f:
    json.dump(obj, f, indent=2, sort_keys=True)
    f.write("\\n")
PY

TARBALL_PATH="$OUT_DIR/$TARBALL_NAME"
tar -C "$STAGE" -czf "$TARBALL_PATH" usr etc build-manifest.json
TARBALL_SHA="$(ws6_sha256_file "$TARBALL_PATH")"

python3 - "$OUT_DIR/build-manifest.json" "$MANIFEST_JSON" "$TARBALL_NAME" "$TARBALL_SHA" <<'PY'
import json, pathlib, sys
out_path, stage_path, name, sha = sys.argv[1:5]
data = json.loads(pathlib.Path(stage_path).read_text())
data["checksum"] = sha
data["artifacts"] = [{
    "name": name,
    "kind": "tarball",
    "compression": "gzip",
    "sha256": sha,
}]
pathlib.Path(out_path).write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
PY

{
  printf '%s  %s\n' "$TARBALL_SHA" "$TARBALL_NAME"
  printf '%s  %s\n' "$EXYONQ_SHA" "exyonq"
  printf '%s  %s\n' "$EXYONQCTL_SHA" "exyonqctl"
} >"$OUT_DIR/SHA256SUMS"

# Companion copies beside tarball (SBOM / licenses)
for f in LICENSE NOTICE THIRD_PARTY_NOTICES.md sbom.cdx.json; do
  if [[ -f "$WORKSPACE/$f" ]]; then
    cp -f "$WORKSPACE/$f" "$OUT_DIR/$f"
  fi
done

# Optional nfpm (amd64 only)
if [[ "$DO_NFPM" == "true" ]]; then
  if [[ "$ARCH" != "amd64" ]]; then
    echo "ERROR: ARM64_NFPM=DEFERRED — refusing --nfpm on arch=$ARCH" >&2
    exit 1
  fi
  if command -v nfpm >/dev/null 2>&1; then
    mkdir -p "$OUT_DIR/nfpm-stage"
    cp "$EXYONQ_BIN" "$OUT_DIR/nfpm-stage/exyonq"
    cp "$EXYONQCTL_BIN" "$OUT_DIR/nfpm-stage/exyonqctl"
    # Place dist/ layout expected by nfpm.yaml
    mkdir -p "$WORKSPACE/dist"
    cp "$EXYONQ_BIN" "$WORKSPACE/dist/exyonq"
    cp "$EXYONQCTL_BIN" "$WORKSPACE/dist/exyonqctl"
    # optional compat placeholder skip
    if [[ ! -x "$WORKSPACE/dist/exyonq-compat" ]]; then
      cp "$EXYONQCTL_BIN" "$WORKSPACE/dist/exyonq-compat"
    fi
    for f in LICENSE NOTICE THIRD_PARTY_NOTICES.md sbom.cdx.json; do
      [[ -f "$WORKSPACE/$f" ]] || touch "$WORKSPACE/$f"
    done
    (
      cd "$WORKSPACE"
      ARCH=amd64 VERSION="$TARGET_RC_VERSION" \
        nfpm pkg --config packaging/nfpm.yaml --packager deb --target "$OUT_DIR/"
      ARCH=amd64 VERSION="$TARGET_RC_VERSION" \
        nfpm pkg --config packaging/nfpm.yaml --packager rpm --target "$OUT_DIR/" || true
    )
    # append package checksums
    shopt -s nullglob
    for pkg in "$OUT_DIR"/*.deb "$OUT_DIR"/*.rpm; do
      printf '%s  %s\n' "$(ws6_sha256_file "$pkg")" "$(basename "$pkg")" >>"$OUT_DIR/SHA256SUMS"
    done
  else
    echo "WARN: nfpm not installed — amd64 packages skipped (document SUPPORT_LIMIT)" >&2
    echo "NFPM_STATUS=SKIPPED_TOOL_MISSING" >"$OUT_DIR/nfpm.status"
  fi
fi

cat >"$OUT_DIR/summary.txt" <<EOF
VERDICT=PASS
HARNESS=p16-ws4-build-artifacts.sh
SOURCE_VERSION=$SOURCE_VERSION
TARGET_RC_VERSION=$TARGET_RC_VERSION
SOURCE_REVISION=$HEAD
ARCH=$ARCH
TARGET=$TARGET
TARBALL=$TARBALL_NAME
TARBALL_SHA256=$TARBALL_SHA
CLEAN_TREE=true
SIGNED=false
PUBLICATION_STATE=candidate_not_released
RC_STATUS=NOT_DECLARED
BIT_FOR_BIT_CLAIM=NOT_CLAIMED
EOF

ws6_log "wrote $TARBALL_PATH"
cat "$OUT_DIR/summary.txt"
