#!/usr/bin/env bash
# ExyonQ install script — downloads a versioned Linux tarball (amd64/arm64)
# and verifies SHA-256 before install. P1.5-WS6: INSTALLER_SHA256_VERIFY=REQUIRED.
set -euo pipefail

REPO="${EXYONQ_INSTALL_REPO:-ExyonQ/ExyonQ}"
VERSION="${EXYONQ_INSTALL_VERSION:-0.3.2.1}"
INSTALL_DIR="${EXYONQ_INSTALL_DIR:-/usr/local/bin}"
# Optional override: local artifact dir (offline / harness). When set, no download.
LOCAL_ARTIFACT_DIR="${EXYONQ_INSTALL_ARTIFACT_DIR:-}"
# Optional: path to SHA256SUMS.txt (or a single .sha256 next to the tarball).
CHECKSUMS_URL="${EXYONQ_INSTALL_CHECKSUMS_URL:-}"
SKIP_VERIFY="${EXYONQ_INSTALL_SKIP_SHA256:-0}"

arch="$(uname -m)"
case "$arch" in
  x86_64) ARCH_LABEL="amd64" ;;
  aarch64|arm64) ARCH_LABEL="arm64" ;;
  *)
    echo "Unsupported architecture: $arch (use cargo install from source)" >&2
    exit 1
    ;;
esac

VERSION_STRIPPED="${VERSION#v}"
# Preferred WS6 naming: exyonq-<version>-linux-<arch>.tar.gz
# Legacy CI naming still accepted when EXYONQ_INSTALL_LEGACY_NAME=1.
if [[ "${EXYONQ_INSTALL_LEGACY_NAME:-0}" == "1" ]]; then
  TARBALL_NAME="exyonq-linux-${ARCH_LABEL}.tar.gz"
else
  TARBALL_NAME="exyonq-${VERSION_STRIPPED}-linux-${ARCH_LABEL}.tar.gz"
fi

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

if [[ -n "$LOCAL_ARTIFACT_DIR" ]]; then
  src="${LOCAL_ARTIFACT_DIR}/${TARBALL_NAME}"
  if [[ ! -f "$src" ]]; then
    # Fall back to legacy name in local artifact dir.
    src="${LOCAL_ARTIFACT_DIR}/exyonq-linux-${ARCH_LABEL}.tar.gz"
  fi
  [[ -f "$src" ]] || {
    echo "ERROR: artifact not found under ${LOCAL_ARTIFACT_DIR}" >&2
    exit 1
  }
  cp "$src" "${tmpdir}/exyonq.tgz"
  if [[ -f "${LOCAL_ARTIFACT_DIR}/SHA256SUMS.txt" ]]; then
    cp "${LOCAL_ARTIFACT_DIR}/SHA256SUMS.txt" "${tmpdir}/SHA256SUMS.txt"
  fi
else
  if [[ "$VERSION" == "latest" ]]; then
    echo "ERROR: EXYONQ_INSTALL_VERSION=latest is forbidden for WS6 evidence (ambiguous)." >&2
    echo "Pin an explicit version (e.g. 0.3.2.1)." >&2
    exit 2
  fi
  base="https://github.com/${REPO}/releases/download/v${VERSION_STRIPPED}"
  url="${base}/${TARBALL_NAME}"
  echo "Downloading ${url}..."
  if ! curl -fsSL "$url" -o "${tmpdir}/exyonq.tgz"; then
    # Legacy release asset name fallback.
    legacy_url="${base}/exyonq-linux-${ARCH_LABEL}.tar.gz"
    echo "Retrying legacy name: ${legacy_url}..."
    curl -fsSL "$legacy_url" -o "${tmpdir}/exyonq.tgz"
    TARBALL_NAME="exyonq-linux-${ARCH_LABEL}.tar.gz"
  fi
  sums_url="${CHECKSUMS_URL:-${base}/SHA256SUMS.txt}"
  if curl -fsSL "$sums_url" -o "${tmpdir}/SHA256SUMS.txt"; then
    :
  else
    echo "WARNING: could not download SHA256SUMS.txt from ${sums_url}" >&2
  fi
fi

if [[ "$SKIP_VERIFY" == "1" ]]; then
  echo "ERROR: EXYONQ_INSTALL_SKIP_SHA256=1 is not a supported install path (P1.5-WS6)." >&2
  echo "Checksum verification is required. Use EXYONQ_INSTALL_ARTIFACT_DIR with SHA256SUMS.txt for offline installs." >&2
  exit 2
elif [[ -f "${tmpdir}/SHA256SUMS.txt" ]]; then
  actual="$(sha256sum "${tmpdir}/exyonq.tgz" 2>/dev/null | awk '{print $1}' \
    || shasum -a 256 "${tmpdir}/exyonq.tgz" | awk '{print $1}')"
  # Match by basename; tolerate accidental path prefixes in older sums files.
  expected="$(awk -v n="$TARBALL_NAME" '
    $1 ~ /^[0-9a-fA-F]{64}$/ {
      f=$2; sub(/^\*/, "", f);
      gsub(/.*\//, "", f);
      if (f == n) { print $1; exit }
    }
  ' "${tmpdir}/SHA256SUMS.txt")"
  if [[ -z "$expected" ]]; then
    echo "ERROR: no SHA-256 entry for ${TARBALL_NAME} in SHA256SUMS.txt" >&2
    cat "${tmpdir}/SHA256SUMS.txt" >&2 || true
    exit 1
  fi
  if [[ "$expected" != "$actual" ]]; then
    echo "ERROR: SHA-256 mismatch for ${TARBALL_NAME}" >&2
    echo "  expected=${expected}" >&2
    echo "  actual=${actual}" >&2
    exit 1
  fi
  echo "SHA-256 OK (${actual})"
else
  echo "ERROR: SHA256SUMS.txt missing — refusing install." >&2
  echo "Provide SHA256SUMS.txt next to the artifact or via EXYONQ_INSTALL_CHECKSUMS_URL." >&2
  exit 1
fi
tar -xzf "${tmpdir}/exyonq.tgz" -C "$tmpdir"

# Layout may be flat (legacy) or usr/bin/ (WS6 staged tarball).
bin_exyonq=""
bin_ctl=""
if [[ -x "${tmpdir}/usr/bin/exyonq" ]]; then
  bin_exyonq="${tmpdir}/usr/bin/exyonq"
  bin_ctl="${tmpdir}/usr/bin/exyonqctl"
elif [[ -x "${tmpdir}/exyonq" ]]; then
  bin_exyonq="${tmpdir}/exyonq"
  bin_ctl="${tmpdir}/exyonqctl"
else
  echo "ERROR: exyonq binary not found in tarball" >&2
  exit 1
fi

install -m 0755 "$bin_exyonq" "${INSTALL_DIR}/exyonq"
if [[ -n "$bin_ctl" && -x "$bin_ctl" ]]; then
  install -m 0755 "$bin_ctl" "${INSTALL_DIR}/exyonqctl"
fi
echo "Installed exyonq to ${INSTALL_DIR}/exyonq"
echo "Run: exyonq validate -c /etc/exyonq/config.toml && exyonq serve -c /etc/exyonq/config.toml"
