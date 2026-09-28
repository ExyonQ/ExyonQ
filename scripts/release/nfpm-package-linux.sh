#!/usr/bin/env bash
# Package prebuilt Linux binaries into .deb and .rpm via nfpm (no binary execution).
# Model: qualified arch binary → nfpm arch package.
# Cap062 / release.yml shared helper.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

usage() {
  cat <<'EOF'
Usage:
  nfpm-package-linux.sh \
    --arch amd64|arm64 \
    --version VERSION \
    --bin-dir DIR \
    --out-dir DIR \
    [--nfpm-config packaging/nfpm.yaml]

bin-dir must contain executable: exyonq, exyonqctl, exyonq-compat
Repo must contain: LICENSE NOTICE THIRD_PARTY_NOTICES.md sbom.cdx.json
and packaging/{config,systemd,tmpfiles.d,logrotate.d,scripts}/ as referenced by nfpm.yaml
EOF
}

ARCH=""
VERSION=""
BIN_DIR=""
OUT_DIR=""
NFPM_CONFIG="packaging/nfpm.yaml"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    --bin-dir) BIN_DIR="$2"; shift 2 ;;
    --out-dir) OUT_DIR="$2"; shift 2 ;;
    --nfpm-config) NFPM_CONFIG="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown arg: $1" >&2; usage >&2; exit 2 ;;
  esac
done

case "$ARCH" in
  amd64|arm64) ;;
  *) echo "ERROR: --arch must be amd64 or arm64 (got '$ARCH')" >&2; exit 2 ;;
esac
[[ -n "$VERSION" && -n "$BIN_DIR" && -n "$OUT_DIR" ]] || { usage >&2; exit 2; }
[[ -f "$ROOT/$NFPM_CONFIG" ]] || { echo "ERROR: missing $ROOT/$NFPM_CONFIG" >&2; exit 1; }
command -v nfpm >/dev/null 2>&1 || { echo "ERROR: nfpm not on PATH" >&2; exit 1; }

BIN_DIR="$(cd "$BIN_DIR" && pwd)"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

for b in exyonq exyonqctl exyonq-compat; do
  [[ -f "$BIN_DIR/$b" ]] || { echo "ERROR: missing $BIN_DIR/$b" >&2; exit 1; }
done

for f in LICENSE NOTICE THIRD_PARTY_NOTICES.md sbom.cdx.json; do
  [[ -f "$ROOT/$f" ]] || { echo "ERROR: missing $f" >&2; exit 1; }
done

STAGE="$(mktemp -d)"
cleanup() { rm -rf "$STAGE"; }
trap cleanup EXIT INT TERM

# Isolated tree so nfpm relative paths resolve without mutating the worktree.
mkdir -p "$STAGE/dist" "$STAGE/packaging"
cp -a "$ROOT/packaging/." "$STAGE/packaging/"
cp -f "$ROOT/LICENSE" "$ROOT/NOTICE" "$ROOT/THIRD_PARTY_NOTICES.md" "$ROOT/sbom.cdx.json" "$STAGE/"
install -m 0755 "$BIN_DIR/exyonq" "$BIN_DIR/exyonqctl" "$BIN_DIR/exyonq-compat" "$STAGE/dist/"

export VERSION ARCH
(
  cd "$STAGE"
  nfpm pkg --config packaging/nfpm.yaml --packager deb --target "$OUT_DIR/"
  nfpm pkg --config packaging/nfpm.yaml --packager rpm --target "$OUT_DIR/"
)

shopt -s nullglob
DEB_CANDIDATES=("$OUT_DIR"/*.deb)
RPM_CANDIDATES=("$OUT_DIR"/*.rpm)
[[ ${#DEB_CANDIDATES[@]} -ge 1 && ${#RPM_CANDIDATES[@]} -ge 1 ]] || {
  echo "ERROR: nfpm did not produce deb+rpm in $OUT_DIR" >&2
  ls -la "$OUT_DIR" >&2 || true
  exit 1
}

# Prefer packages whose name encodes this arch.
pick_arch_pkg() {
  local kind="$1"
  local f base
  for f in "$OUT_DIR"/*."$kind"; do
    [[ -f "$f" ]] || continue
    base="$(basename "$f")"
    case "$ARCH" in
      amd64)
        if echo "$base" | grep -Eiq 'amd64|x86_64'; then
          printf '%s\n' "$f"
          return 0
        fi
        ;;
      arm64)
        if echo "$base" | grep -Eiq 'arm64|aarch64'; then
          printf '%s\n' "$f"
          return 0
        fi
        ;;
    esac
  done
  # Fallback: newest of kind (single-arch out dir)
  ls -t "$OUT_DIR"/*."$kind" 2>/dev/null | head -n1
}

DEB="$(pick_arch_pkg deb)"
RPM="$(pick_arch_pkg rpm)"
[[ -n "$DEB" && -n "$RPM" && -f "$DEB" && -f "$RPM" ]] || {
  echo "ERROR: could not select arch-matching deb/rpm" >&2
  exit 1
}

expect_elf_re() {
  case "$ARCH" in
    amd64) printf 'x86-64|x86_64|Intel 80386' ;;
    arm64) printf 'aarch64|ARM aarch64' ;;
  esac
}

verify_deb_elf() {
  local pkg="$1"
  local tmp extract_bin file_out
  tmp="$(mktemp -d)"
  if command -v dpkg-deb >/dev/null 2>&1; then
    dpkg-deb -x "$pkg" "$tmp"
  else
    (cd "$tmp" && ar x "$pkg")
    local data
    data="$(ls "$tmp"/data.tar.* | head -n1)"
    tar -xf "$data" -C "$tmp"
  fi
  extract_bin="$tmp/usr/bin/exyonq"
  [[ -x "$extract_bin" || -f "$extract_bin" ]] || {
    echo "ERROR: packaged /usr/bin/exyonq missing in $(basename "$pkg")" >&2
    rm -rf "$tmp"
    return 1
  }
  file_out="$(file -b "$extract_bin" || true)"
  if ! echo "$file_out" | grep -Eiq "$(expect_elf_re)"; then
    echo "ERROR: DEB ELF arch mismatch ARCH=$ARCH file='$file_out' pkg=$(basename "$pkg")" >&2
    rm -rf "$tmp"
    return 1
  fi
  rm -rf "$tmp"
}

verify_rpm_elf() {
  local pkg="$1"
  local tmp extract_bin file_out
  if ! command -v rpm2cpio >/dev/null 2>&1; then
    if command -v apt-get >/dev/null 2>&1; then
      sudo -n apt-get install -y rpm2cpio cpio >/dev/null 2>&1 \
        || apt-get install -y rpm2cpio cpio >/dev/null 2>&1 \
        || true
    fi
  fi
  if ! command -v rpm2cpio >/dev/null 2>&1 || ! command -v cpio >/dev/null 2>&1; then
    echo "ERROR: rpm2cpio/cpio required to verify RPM ELF payload" >&2
    return 1
  fi
  tmp="$(mktemp -d)"
  # RPM payloads use absolute member names; strip leading '/' into $tmp.
  # cpio may return non-zero on warnings — do not let pipefail abort packaging.
  set +e
  (cd "$tmp" && rpm2cpio "$pkg" | cpio -idm --no-absolute-filenames >/dev/null 2>&1)
  set -e
  extract_bin="$tmp/usr/bin/exyonq"
  [[ -f "$extract_bin" ]] || {
    echo "ERROR: packaged /usr/bin/exyonq missing in $(basename "$pkg")" >&2
    rm -rf "$tmp"
    return 1
  }
  file_out="$(file -b "$extract_bin" || true)"
  if ! echo "$file_out" | grep -Eiq "$(expect_elf_re)"; then
    echo "ERROR: RPM ELF arch mismatch ARCH=$ARCH file='$file_out' pkg=$(basename "$pkg")" >&2
    rm -rf "$tmp"
    return 1
  fi
  rm -rf "$tmp"
}

# Filename must agree with ARCH (no opposite-arch label).
for pkg in "$DEB" "$RPM"; do
  base="$(basename "$pkg")"
  if [[ "$ARCH" == amd64 ]]; then
    if echo "$base" | grep -Eiq 'arm64|aarch64'; then
      echo "ERROR: amd64 package filename claims arm: $base" >&2
      exit 1
    fi
  else
    if echo "$base" | grep -Eiq '(^|[^a-z])(amd64|x86_64)([^a-z]|$)' && ! echo "$base" | grep -Eiq 'arm64|aarch64'; then
      echo "ERROR: arm64 package filename claims amd64: $base" >&2
      exit 1
    fi
  fi
done

verify_deb_elf "$DEB"
verify_rpm_elf "$RPM"

echo "NFPM_OK arch=$ARCH version=$VERSION deb=$(basename "$DEB") rpm=$(basename "$RPM")"
echo "DEB_PATH=$DEB"
echo "RPM_PATH=$RPM"
