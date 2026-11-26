#!/usr/bin/env bash
# NG-21 — cross-check that the core crate compiles for both Linux targets.
#
# A foreign Linux target has no libc headers on macOS, so the check needs a
# sysroot. This script builds one from pinned Ubuntu .debs (the recipe recorded
# for the 2026-11-15 run, made reproducible), then runs `cargo check` per target
# with a dedicated CARGO_TARGET_DIR so a warm cache cannot hide a missing
# sysroot.
#
# Every .deb is pinned by SHA-256, the value published in the Ubuntu
# noble-updates/main Packages index. A cached or downloaded file whose hash
# does not match is discarded and fetched again once; a second mismatch aborts
# the gate instead of unpacking bytes the network supplied unverified.
#
# Usage:
#   bash scripts/gates/linux-cross-check.sh                  # both Linux targets
#   bash scripts/gates/linux-cross-check.sh --target aarch64-unknown-linux-gnu
#   bash scripts/gates/linux-cross-check.sh --sysroot DIR --no-download
#   bash scripts/gates/linux-cross-check.sh --package exyonq-core --target-dir /tmp/x
#
# Exit: 0 = every requested target check passed; 1 = at least one target failed,
#       including a sysroot that could not be prepared; 2 = bad usage.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# The sysroot is assembled from these two packages: "<name> <version> <pool>".
# Bump a version here and its SHA-256 in deb_sha256() together.
PINNED_PACKAGES='libc6-dev 2.39-0ubuntu8.12 g/glibc
linux-libc-dev 6.8.0-151.151 l/linux'

PACKAGE=exyonq-core
TARGETS=""
SYSROOT=""
SYSROOT_DIR=""
TARGET_DIR="$ROOT/target/ng21-cross"
DOWNLOADS=1

need_value() {
  [ -n "${2:-}" ] || {
    echo "linux-cross-check: $1 requires a value" >&2
    exit 2
  }
}

while [ $# -gt 0 ]; do
  case "$1" in
    --target)
      need_value --target "${2:-}"
      TARGETS="$TARGETS $2"
      shift
      ;;
    --package)
      need_value --package "${2:-}"
      PACKAGE="$2"
      shift
      ;;
    --sysroot)
      need_value --sysroot "${2:-}"
      SYSROOT="$2"
      shift
      ;;
    --target-dir)
      need_value --target-dir "${2:-}"
      TARGET_DIR="$2"
      shift
      ;;
    --no-download) DOWNLOADS=0 ;;
    -h | --help)
      sed -n '2,/^set -/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "linux-cross-check: unknown argument '$1'" >&2
      exit 2
      ;;
  esac
  shift
done

if [ -z "$TARGETS" ]; then
  TARGETS="x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu"
fi
if [ -z "$SYSROOT" ]; then
  SYSROOT="$ROOT/target/sysroot"
fi
case "$SYSROOT" in
  /*) ;;
  *) SYSROOT="$ROOT/$SYSROOT" ;;
esac
case "$TARGET_DIR" in
  /*) ;;
  *) TARGET_DIR="$ROOT/$TARGET_DIR" ;;
esac

command -v curl >/dev/null 2>&1 || {
  echo "linux-cross-check: curl is required to fetch the sysroot" >&2
  exit 2
}
command -v ar >/dev/null 2>&1 || {
  echo "linux-cross-check: ar is required to unpack the .debs" >&2
  exit 2
}
command -v sha256sum >/dev/null 2>&1 || command -v shasum >/dev/null 2>&1 || {
  echo "linux-cross-check: sha256sum or shasum is required to verify the .deb pins" >&2
  exit 2
}

triple_arch() {
  case "$1" in
    x86_64-unknown-linux-gnu) echo amd64 ;;
    aarch64-unknown-linux-gnu) echo arm64 ;;
    *) return 1 ;;
  esac
}

triple_multiarch() {
  case "$1" in
    x86_64-unknown-linux-gnu) echo x86_64-linux-gnu ;;
    aarch64-unknown-linux-gnu) echo aarch64-linux-gnu ;;
    *) return 1 ;;
  esac
}

# amd64 lives on archive.ubuntu.com, arm64 on the ports host.
debs_host() {
  case "$1" in
    amd64) echo http://archive.ubuntu.com/ubuntu ;;
    arm64) echo http://ports.ubuntu.com/ubuntu-ports ;;
    *) return 1 ;;
  esac
}

# SHA-256 of each pinned .deb, copied from the `SHA256:` field of the Ubuntu
# noble-updates/main Packages index and cross-checked on 2026-11-21 against
#   archive.ubuntu.com/ubuntu/dists/noble-updates/main/binary-amd64/Packages.xz
#   ports.ubuntu.com/ubuntu-ports/dists/noble-updates/main/binary-arm64/Packages.xz
deb_sha256() {
  case "$1/$2" in
    libc6-dev/amd64) echo d81dd6dc9d515762cc93f338efa6219252acf3e2e87b5b89dd7dc517741e6eed ;;
    libc6-dev/arm64) echo b8411b504a468c19a2acf4ff38a2015997853d078138afac76ff89c9599794b2 ;;
    linux-libc-dev/amd64) echo badd744a9dc7e43c53975ac08d88e23dd13223f2334637c25fd9b9e9bb5d236d ;;
    linux-libc-dev/arm64) echo e9fe154b8357bfdba61fe4f95690be4aaccdc8fe14e08571dc396633185a170f ;;
    *) return 1 ;;
  esac
}

# The pins a sysroot was built from. Recording them means a later version bump
# cannot silently reuse a sysroot assembled from the previous .debs.
pin_stamp() {
  local arch="$1" name version pool
  while read -r name version pool; do
    [ -n "$name" ] || continue
    echo "$name $version $(deb_sha256 "$name" "$arch")"
  done <<EOF
$PINNED_PACKAGES
EOF
}

# Sysroots built here carry the stamp; the 2026-11-15 recipe predates it and
# those stay reusable as they are.
pins_match() {
  local dir="$1" arch="$2"
  [ -s "$dir/.pinned-debs" ] || return 0
  [ "$(cat "$dir/.pinned-debs")" = "$(pin_stamp "$arch")" ]
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

# Verify one .deb against its pin. Returns 0 only on an exact match; a mismatch
# removes the file so the caller can download it again.
verify_deb() {
  local path="$1" package="$2" arch="$3" expected actual
  expected="$(deb_sha256 "$package" "$arch")" || {
    echo "linux-cross-check: no pinned sha256 for $package/$arch" >&2
    return 1
  }
  actual="$(sha256_file "$path")" || return 1
  if [ "$actual" != "$expected" ]; then
    echo "linux-cross-check: sha256 mismatch for $(basename "$path"): got $actual expected $expected" >&2
    rm -f "$path"
    return 1
  fi
  return 0
}

# A complete sysroot needs both packages: a glibc header and the linux UAPI tree.
sysroot_ready() {
  [ -f "$1/usr/include/sys/sendfile.h" ] && [ -f "$1/usr/include/linux/socket.h" ]
}

unpack_deb() {
  local deb="$1" dest="$2" tmp member
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-deb.XXXXXX")" || return 1
  (
    cd "$tmp" || exit 1
    ar x "$deb" || exit 1
    member="$(ls data.tar.* 2>/dev/null | head -n 1)"
    [ -n "$member" ] || {
      echo "linux-cross-check: $deb has no data.tar.* member" >&2
      exit 1
    }
    # bsdtar and GNU tar both detect the compression from the member's magic.
    tar -xf "$member" -C "$dest" || exit 1
  )
  local rc=$?
  rm -rf "$tmp"
  return $rc
}

link_multiarch_headers() {
  local dir="$1" multi="$2" entry name
  for entry in "$dir/usr/include/$multi"/*; do
    [ -e "$entry" ] || continue
    name="$(basename "$entry")"
    if [ ! -e "$dir/usr/include/$name" ]; then
      ln -s "$multi/$name" "$dir/usr/include/$name"
    fi
  done
}

# Reuse whichever layout already holds a complete sysroot: the 2026-11-15 run
# used <arch>-linux, this script creates <arch>-linux-gnu.
sysroot_dir_for() {
  local multi="$1" legacy="$SYSROOT/${1%-gnu}" candidate
  for candidate in "$legacy" "$SYSROOT/$multi"; do
    if sysroot_ready "$candidate"; then
      echo "$candidate"
      return 0
    fi
  done
  echo "$SYSROOT/$multi"
}

# Reuse a verified cached .deb, otherwise download it and verify the bytes
# before they can reach the sysroot.
fetch_deb() {
  local path="$1" package="$2" arch="$3" url="$4" name
  name="$(basename "$path")"
  if [ -f "$path" ] && verify_deb "$path" "$package" "$arch"; then
    return 0
  fi
  if [ -f "$path" ]; then
    echo "linux-cross-check: cached $name rejected; fetching it again" >&2
  fi
  echo "linux-cross-check: downloading $url"
  if ! curl -fsSL -o "$path" "$url"; then
    rm -f "$path"
    echo "linux-cross-check: download failed: $url" >&2
    return 1
  fi
  verify_deb "$path" "$package" "$arch" || {
    echo "linux-cross-check: $name failed sha256 verification; refusing to unpack it" >&2
    return 1
  }
}

# Prepare the sysroot for one target and leave it in SYSROOT_DIR.
prepare_sysroot() {
  local triple="$1" arch multi dir name version pool deb
  arch="$(triple_arch "$triple")" || return 1
  multi="$(triple_multiarch "$triple")" || return 1
  dir="$(sysroot_dir_for "$multi")"
  if sysroot_ready "$dir"; then
    if pins_match "$dir" "$arch"; then
      echo "linux-cross-check: sysroot for $triple already present ($dir)"
      SYSROOT_DIR="$dir"
      return 0
    fi
    if [ "$DOWNLOADS" -eq 0 ]; then
      echo "linux-cross-check: sysroot at $dir was built from different pins and --no-download was given" >&2
      return 1
    fi
    echo "linux-cross-check: sysroot at $dir was built from different pins; rebuilding it" >&2
    rm -rf "$dir"
  elif [ "$DOWNLOADS" -eq 0 ]; then
    echo "linux-cross-check: sysroot for $triple is missing at $dir and --no-download was given" >&2
    return 1
  fi
  mkdir -p "$dir" "$SYSROOT/debs" || return 1
  while read -r name version pool; do
    [ -n "$name" ] || continue
    deb="${name}_${version}_${arch}.deb"
    fetch_deb "$SYSROOT/debs/$deb" "$name" "$arch" \
      "$(debs_host "$arch")/pool/main/$pool/$deb" || return 1
    unpack_deb "$SYSROOT/debs/$deb" "$dir" || return 1
  done <<EOF
$PINNED_PACKAGES
EOF
  link_multiarch_headers "$dir" "$multi" || return 1
  sysroot_ready "$dir" || return 1
  pin_stamp "$arch" >"$dir/.pinned-debs"
  SYSROOT_DIR="$dir"
}

fail=0
for triple in $TARGETS; do
  multi="$(triple_multiarch "$triple")" || {
    echo "linux-cross-check: unsupported target '$triple'" >&2
    exit 2
  }
  if ! rustup target list --installed 2>/dev/null | grep -qx "$triple"; then
    echo "linux-cross-check: missing rustup target '$triple' (rustup target add $triple)" >&2
    exit 2
  fi
  if ! prepare_sysroot "$triple"; then
    echo "FAIL $triple: sysroot unavailable at $SYSROOT/$multi" >&2
    fail=1
    continue
  fi

  dir="$SYSROOT_DIR"
  # cc-rs looks the target up with dashes replaced by underscores, preserving
  # case (CC_x86_64_unknown_linux_gnu); an uppercased name is not recognised
  # and the C dependencies then compile without --sysroot.
  triple_env="$(echo "$triple" | tr '-' '_')"
  echo "linux-cross-check: cargo check -p $PACKAGE --target $triple"
  env "CC_$triple_env=clang" \
    "CFLAGS_$triple_env=--sysroot=$dir -isystem $dir/usr/include/$multi" \
    CARGO_TARGET_DIR="$TARGET_DIR" \
    cargo check -p "$PACKAGE" --target "$triple" --locked
  rc=$?

  case "$(triple_arch "$triple")" in
    amd64) echo "LINUX_CHECK_EXIT=$rc" ;;
    arm64) echo "LINUX_ARM_CHECK_EXIT=$rc" ;;
  esac
  if [ "$rc" -ne 0 ]; then
    echo "FAIL $triple: cargo check exited $rc" >&2
    fail=1
  else
    echo "PASS $triple"
  fi
done

if [ "$fail" -eq 0 ]; then
  echo "LINUX_CROSS_CHECK=PASS"
else
  echo "LINUX_CROSS_CHECK=FAIL"
fi
exit "$fail"
