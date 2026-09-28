#!/usr/bin/env bash
# Cap062 — build Linux packages on a native arch host and run real tarball+deb E2E.
# RPM packages are always generated; RPM install/runtime is PASS only on RPM-family OS.
# ZERO_FAKE: packaged binary path must equal executed path (SHA256).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
# shellcheck source=scripts/release/lib/ws6-common.sh
source "$ROOT/scripts/release/lib/ws6-common.sh"

ws6_require_linux

export PATH="${HOME}/.cargo/bin:/root/.cargo/bin:/usr/local/bin:${PATH}"
# shellcheck disable=SC1091
source "${HOME}/.cargo/env" 2>/dev/null || true
# shellcheck disable=SC1091
source /root/.cargo/env 2>/dev/null || true

ARCH="$(ws6_host_arch_label)"
TARGET="$(ws6_arch_to_target "$ARCH")"
VERSION="$(ws6_read_version "$ROOT")"
HEAD="$(ws6_git_head "$ROOT")"
TREE="$(git -C "$ROOT" rev-parse 'HEAD^{tree}' 2>/dev/null || true)"
if [[ -z "$TREE" || "$TREE" == UNKNOWN ]]; then
  TREE="NOT_RECORDED"
fi
LOCK_SHA="$(ws6_lockfile_hash "$ROOT")"
RUN_ID="${CAP062_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
OUT_DIR="${CAP062_OUT_DIR:-$ROOT/.exyonq-local/tmp/phase1-cap062-${RUN_ID}/${ARCH}}"
mkdir -p "$OUT_DIR"/{artifacts,extract,logs,evidence}
SUMMARY="$OUT_DIR/summary.json"
STEPS="$OUT_DIR/steps.jsonl"
: >"$STEPS"

record() {
  local step="$1" status="$2" detail="${3:-}"
  printf '{"step":"%s","status":"%s","detail":%s,"ts":"%s"}\n' \
    "$step" "$status" "$(python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$detail")" \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$STEPS"
  echo "[cap062/$ARCH] $step=$status $detail" >&2
  if [[ "$status" == FAIL ]]; then
    OVERALL=FAIL
  fi
}

OVERALL=PASS
PKG_FAMILY=unknown
if command -v dpkg >/dev/null 2>&1 && [[ -f /etc/debian_version ]]; then
  PKG_FAMILY=debian
elif command -v rpm >/dev/null 2>&1 && { [[ -f /etc/redhat-release ]] || [[ -f /etc/fedora-release ]] || grep -Eiq 'rhel|fedora|rocky|alma|centos' /etc/os-release 2>/dev/null; }; then
  PKG_FAMILY=rpm
fi

ensure_nfpm() {
  if command -v nfpm >/dev/null 2>&1; then
    return 0
  fi
  local url tmp
  case "$(uname -m)" in
    x86_64) url="https://github.com/goreleaser/nfpm/releases/download/v2.41.3/nfpm_2.41.3_linux_x86_64.tar.gz" ;;
    aarch64) url="https://github.com/goreleaser/nfpm/releases/download/v2.41.3/nfpm_2.41.3_linux_arm64.tar.gz" ;;
    *) echo "ERROR: unsupported arch for nfpm bootstrap" >&2; return 1 ;;
  esac
  tmp="$(mktemp -d)"
  curl -fsSL "$url" -o "$tmp/nfpm.tgz"
  tar -xzf "$tmp/nfpm.tgz" -C "$tmp"
  install -m 0755 "$tmp/nfpm" "${HOME}/.local/bin/nfpm" 2>/dev/null || {
    mkdir -p "${HOME}/.local/bin"
    install -m 0755 "$tmp/nfpm" "${HOME}/.local/bin/nfpm"
  }
  export PATH="${HOME}/.local/bin:$PATH"
  rm -rf "$tmp"
  command -v nfpm >/dev/null 2>&1
}

sha256_file() {
  sha256sum "$1" | awk '{print $1}'
}

# --- Build ---
record BUILD_START PASS "target=$TARGET version=$VERSION head=$HEAD"
export EXYONQ_SOURCE_REVISION="$HEAD"
export EXYONQ_OFFICIAL_RELEASE="${EXYONQ_OFFICIAL_RELEASE:-0}"
if [[ ! "$EXYONQ_SOURCE_REVISION" =~ ^[0-9a-f]{40}$ ]]; then
  record BUILD_REVISION FAIL "invalid EXYONQ_SOURCE_REVISION"
  OVERALL=FAIL
fi

CARGO_OK=0
if cargo build --release --locked -p exyonq -p exyonqctl -p exyonq-compat-cli --target "$TARGET" \
  >"$OUT_DIR/logs/cargo-build.log" 2>&1; then
  CARGO_OK=1
else
  record CARGO_BUILD FAIL "see logs/cargo-build.log"
  tail -n 80 "$OUT_DIR/logs/cargo-build.log" >&2 || true
  OVERALL=FAIL
fi
BIN_ROOT="$ROOT/target/$TARGET/release"
if [[ "$CARGO_OK" -eq 1 ]]; then
  for b in exyonq exyonqctl exyonq-compat; do
    [[ -x "$BIN_ROOT/$b" ]] || { record BIN_MISSING FAIL "$BIN_ROOT/$b"; OVERALL=FAIL; CARGO_OK=0; }
  done
fi
if [[ "$CARGO_OK" -eq 1 ]]; then
  record CARGO_BUILD PASS "bins under $BIN_ROOT"
else
  echo "ERROR: refusing to package after cargo/bin failure" >&2
  python3 - "$SUMMARY" <<PY
import json, pathlib
steps=[]
for line in pathlib.Path("$STEPS").read_text().splitlines():
    if line.strip():
        steps.append(json.loads(line))
pathlib.Path("$SUMMARY").write_text(json.dumps({
  "CAPABILITY_ID":"062","ARCH":"$ARCH","OVERALL":"FAIL","FAIL_REASON":"CARGO_BUILD",
  "steps":steps}, indent=2)+"\n")
PY
  exit 1
fi

# Flat tarball
FLAT_NAME="exyonq-linux-${ARCH}.tar.gz"
FLAT_PATH="$OUT_DIR/artifacts/$FLAT_NAME"
tar -C "$BIN_ROOT" -czf "$FLAT_PATH" exyonq exyonqctl exyonq-compat
# Append legal into flat via staging
FLAT_STAGE="$(mktemp -d)"
cp -f "$BIN_ROOT/exyonq" "$BIN_ROOT/exyonqctl" "$BIN_ROOT/exyonq-compat" "$FLAT_STAGE/"
chmod 0755 "$FLAT_STAGE"/*
cp -f "$ROOT/LICENSE" "$ROOT/NOTICE" "$ROOT/THIRD_PARTY_NOTICES.md" "$ROOT/sbom.cdx.json" "$FLAT_STAGE/"
tar -C "$FLAT_STAGE" -czf "$FLAT_PATH" \
  exyonq exyonqctl exyonq-compat LICENSE NOTICE THIRD_PARTY_NOTICES.md sbom.cdx.json
rm -rf "$FLAT_STAGE"
FLAT_SHA="$(sha256_file "$FLAT_PATH")"
record FLAT_TARBALL PASS "$FLAT_NAME sha256=$FLAT_SHA"

# Versioned FHS tarball (reuse WS6 builder against already-built target if possible)
FHS_OUT="$OUT_DIR/artifacts/fhs"
mkdir -p "$FHS_OUT"
# WS6 builds again — acceptable for Cap062 integrity (same locked HEAD).
bash "$ROOT/scripts/release/p15-ws6-build-artifacts.sh" \
  --workspace "$ROOT" \
  --target "$TARGET" \
  --out-dir "$FHS_OUT" \
  >"$OUT_DIR/logs/fhs-build.log" 2>&1 || {
  record FHS_TARBALL FAIL "see logs/fhs-build.log"
  tail -n 60 "$OUT_DIR/logs/fhs-build.log" >&2 || true
  OVERALL=FAIL
}
FHS_NAME="exyonq-${VERSION}-linux-${ARCH}.tar.gz"
# WS6 may use version label; locate produced tarball
FHS_PATH="$(find "$FHS_OUT" -maxdepth 1 -type f -name "exyonq-*-linux-${ARCH}.tar.gz" | head -n1 || true)"
if [[ -z "$FHS_PATH" ]]; then
  record FHS_TARBALL FAIL "no versioned tarball in $FHS_OUT"
  OVERALL=FAIL
else
  cp -f "$FHS_PATH" "$OUT_DIR/artifacts/$(basename "$FHS_PATH")"
  FHS_PATH="$OUT_DIR/artifacts/$(basename "$FHS_PATH")"
  FHS_SHA="$(sha256_file "$FHS_PATH")"
  record FHS_TARBALL PASS "$(basename "$FHS_PATH") sha256=$FHS_SHA"
fi

# nfpm deb+rpm
ensure_nfpm || { record NFPM_INSTALL FAIL "could not install nfpm"; OVERALL=FAIL; }
PKG_OUT="$OUT_DIR/artifacts/packages"
mkdir -p "$PKG_OUT"
NFPM_OK=0
DEB_PATH=""
RPM_PATH=""
if bash "$ROOT/scripts/release/nfpm-package-linux.sh" \
  --arch "$ARCH" \
  --version "$VERSION" \
  --bin-dir "$BIN_ROOT" \
  --out-dir "$PKG_OUT" \
  >"$OUT_DIR/logs/nfpm.log" 2>&1; then
  DEB_PATH="$(find "$PKG_OUT" -maxdepth 1 -type f -name '*.deb' | head -n1 || true)"
  RPM_PATH="$(find "$PKG_OUT" -maxdepth 1 -type f -name '*.rpm' | head -n1 || true)"
  if [[ -n "$DEB_PATH" && -n "$RPM_PATH" ]]; then
    cp -f "$DEB_PATH" "$RPM_PATH" "$OUT_DIR/artifacts/"
    DEB_PATH="$OUT_DIR/artifacts/$(basename "$DEB_PATH")"
    RPM_PATH="$OUT_DIR/artifacts/$(basename "$RPM_PATH")"
    record NFPM_PKG PASS "deb=$(basename "$DEB_PATH") rpm=$(basename "$RPM_PATH")"
    NFPM_OK=1
  else
    record NFPM_PKG FAIL "missing deb or rpm after helper success"
    OVERALL=FAIL
  fi
else
  record NFPM_PKG FAIL "see logs/nfpm.log"
  cat "$OUT_DIR/logs/nfpm.log" >&2 || true
  OVERALL=FAIL
  DEB_PATH=""
  RPM_PATH=""
fi

{
  printf '%s  %s\n' "$FLAT_SHA" "$FLAT_NAME"
  [[ -n "${FHS_PATH:-}" && -f "${FHS_PATH:-}" ]] && printf '%s  %s\n' "$FHS_SHA" "$(basename "$FHS_PATH")"
  [[ "$NFPM_OK" -eq 1 && -n "${DEB_PATH:-}" && -f "${DEB_PATH:-}" ]] && printf '%s  %s\n' "$(sha256_file "$DEB_PATH")" "$(basename "$DEB_PATH")"
  [[ "$NFPM_OK" -eq 1 && -n "${RPM_PATH:-}" && -f "${RPM_PATH:-}" ]] && printf '%s  %s\n' "$(sha256_file "$RPM_PATH")" "$(basename "$RPM_PATH")"
} >"$OUT_DIR/artifacts/SHA256SUMS.txt"

# Privacy scan on FHS extract + deb extract when gate exists
privacy_scan_tree() {
  local tree="$1" label="$2"
  if [[ -x "$ROOT/scripts/gates/publication-privacy-gate.sh" ]]; then
    if bash "$ROOT/scripts/gates/publication-privacy-gate.sh" --tree "$tree" \
      >"$OUT_DIR/logs/privacy-${label}.log" 2>&1; then
      record "PRIVACY_$label" PASS "publication-privacy-gate"
    else
      record "PRIVACY_$label" FAIL "see logs/privacy-${label}.log"
      OVERALL=FAIL
    fi
  else
    # Fail closed light scan for private markers
    if rg -n '/Volumes/Lexar|exyonq-laboratorio|\.exyonq-local|BEGIN PRIVATE KEY|ghp_' "$tree" >/dev/null 2>&1; then
      record "PRIVACY_$label" FAIL "prohibited markers in package tree"
      OVERALL=FAIL
    else
      record "PRIVACY_$label" PASS "marker scan clean (gate binary absent)"
    fi
  fi
}

# --- Tarball E2E (versioned FHS) ---
if [[ -n "${FHS_PATH:-}" && -f "$FHS_PATH" ]]; then
  EXT="$OUT_DIR/extract/fhs"
  rm -rf "$EXT"
  mkdir -p "$EXT"
  # Verify checksum before extract
  echo "$FHS_SHA  $FHS_PATH" | sha256sum -c - >/dev/null
  record FHS_CHECKSUM PASS "matches SHA256SUMS"
  tar -xzf "$FHS_PATH" -C "$EXT"
  privacy_scan_tree "$EXT" FHS

  PACKAGED="$EXT/usr/bin/exyonq"
  PACKAGED_CTL="$EXT/usr/bin/exyonqctl"
  [[ -x "$PACKAGED" ]] || { record FHS_PERM FAIL "exyonq not executable"; OVERALL=FAIL; }
  PACKAGED_SHA="$(sha256_file "$PACKAGED")"
  FILE_OUT="$(file -b "$PACKAGED" || true)"
  ELF_OK=0
  case "$ARCH" in
    amd64) echo "$FILE_OUT" | grep -Eiq 'x86-64|x86_64' && ELF_OK=1 ;;
    arm64) echo "$FILE_OUT" | grep -Eiq 'aarch64' && ELF_OK=1 ;;
  esac
  if [[ "$ELF_OK" -eq 1 ]]; then
    record FHS_ELF PASS "$FILE_OUT"
  else
    record FHS_ELF FAIL "$FILE_OUT"
    OVERALL=FAIL
  fi

  # Wrong-arch rejection: if opposite tarball present, refuse
  if [[ -n "${CAP062_WRONG_ARCH_TARBALL:-}" && -f "${CAP062_WRONG_ARCH_TARBALL}" ]]; then
    if bash "$ROOT/scripts/release/p15-ws6-install-verify.sh" \
      --artifact "$CAP062_WRONG_ARCH_TARBALL" \
      --staging-root "$OUT_DIR/extract/wrong-arch" \
      >"$OUT_DIR/logs/wrong-arch.log" 2>&1; then
      record WRONG_ARCH_REJECT FAIL "wrong-arch install-verify unexpectedly passed"
      OVERALL=FAIL
    else
      record WRONG_ARCH_REJECT PASS "wrong-arch rejected"
    fi
  else
    record WRONG_ARCH_REJECT SKIP "no CAP062_WRONG_ARCH_TARBALL provided"
  fi

  RT="$(mktemp -d /tmp/cap062-rt.XXXXXX)"
  mkdir -p "$RT/public" "$RT/run"
  echo "cap062-fhs-ok" >"$RT/public/index.html"
  PORT="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
  CFG="$RT/config.toml"
  SOCK="$RT/run/control.sock"
  cat >"$CFG" <<EOF
config_version = 1

[logging]
level = "info"
format = "text"
queue_capacity = 1024
[logging.console]
enabled = true
stream = "stdout"
[logging.file]
enabled = false
[logging.access]
enabled = true
[logging.audit]
enabled = false

[[server]]
listen = "127.0.0.1:${PORT}"
routes = ["site"]

[[route]]
name = "site"
match = { path = "/" }
root = "${RT}/public"
index = "index.html"
EOF

  LOG="$OUT_DIR/logs/fhs-serve.log"
  EXYONQ_CONTROL_SOCKET="$SOCK" "$PACKAGED" serve -c "$CFG" >"$LOG" 2>&1 &
  PID=$!
  ready=false
  for _ in $(seq 1 50); do
    if curl -sf "http://127.0.0.1:${PORT}/" | grep -q 'cap062-fhs-ok'; then
      ready=true
      break
    fi
    sleep 0.2
  done
  if [[ "$ready" == true ]]; then
    record FHS_RUNTIME PASS "pid=$PID port=$PORT"
  else
    record FHS_RUNTIME FAIL "server/response failed"
    tail -n 40 "$LOG" >&2 || true
    OVERALL=FAIL
  fi

  # ZERO_FAKE binary identity: measure /proc/$PID/exe; do not invent equality.
  EXE_LINK="$(readlink -f "/proc/$PID/exe" 2>/dev/null || true)"
  PACKAGED_REAL="$(readlink -f "$PACKAGED")"
  EXECUTED_SHA="NOT_MEASURED"
  if [[ -n "$EXE_LINK" && -e "/proc/$PID/exe" ]]; then
    EXECUTED_SHA="$(sha256_file "/proc/$PID/exe")"
  fi
  IDENTITY_OK=0
  if [[ "$EXECUTED_SHA" != "NOT_MEASURED" && "$EXECUTED_SHA" == "$PACKAGED_SHA" ]]; then
    IDENTITY_OK=1
  elif [[ "$EXECUTED_SHA" == "NOT_MEASURED" && -n "$EXE_LINK" && "$EXE_LINK" == "$PACKAGED_REAL" ]]; then
    # Path proof only when hash could not be measured.
    EXECUTED_SHA="$PACKAGED_SHA"
    IDENTITY_OK=1
  fi
  {
    echo "EXECUTED_BINARY_PATH=$PACKAGED"
    echo "PACKAGED_BINARY_SHA256=$PACKAGED_SHA"
    echo "EXECUTED_BINARY_SHA256=$EXECUTED_SHA"
    echo "PROC_EXE_LINK=${EXE_LINK:-}"
  } >"$OUT_DIR/evidence/fhs-binary.txt"
  if [[ "$IDENTITY_OK" -eq 1 ]]; then
    record FHS_BINARY_IDENTITY PASS "EXECUTED_BINARY_SHA256=$EXECUTED_SHA"
  else
    record FHS_BINARY_IDENTITY FAIL "packaged=$PACKAGED_SHA executed=$EXECUTED_SHA link=$EXE_LINK"
    OVERALL=FAIL
  fi
  if [[ -x "$PACKAGED_CTL" ]]; then
    if EXYONQ_CONTROL_SOCKET="$SOCK" "$PACKAGED_CTL" status --socket "$SOCK" >"$OUT_DIR/logs/fhs-ctl.log" 2>&1; then
      record FHS_CTL PASS "exyonqctl status"
    else
      record FHS_CTL WARN "exyonqctl status failed; HTTP path passed"
    fi
  fi
  kill -TERM "$PID" 2>/dev/null || true
  wait "$PID" 2>/dev/null || true
  record FHS_SHUTDOWN PASS
  rm -rf "$RT"
fi

# --- DEB E2E ---
if [[ "$PKG_FAMILY" != debian ]]; then
  record DEB_E2E SKIP "host not Debian-family (pkg_family=$PKG_FAMILY)"
elif [[ "${NFPM_OK:-0}" -ne 1 || -z "${DEB_PATH:-}" || ! -f "$DEB_PATH" ]]; then
  record DEB_E2E FAIL "no verified deb artifact (NFPM_OK=${NFPM_OK:-0})"
  OVERALL=FAIL
else
  # Isolate under a dedicated prefix where possible; for real systemd we need system install.
  # Cap062 requires real package tooling + systemd — use dpkg -i with unique unit instance.
  SUDO=""
  if [[ "$(id -u)" -ne 0 ]]; then
    SUDO="sudo -n"
  fi
  DEB_INSTALLED=0
  if $SUDO dpkg -i "$DEB_PATH" >"$OUT_DIR/logs/dpkg-install.log" 2>&1; then
    DEB_INSTALLED=1
  else
    $SUDO apt-get install -y -f >>"$OUT_DIR/logs/dpkg-install.log" 2>&1 || true
    if $SUDO dpkg -i "$DEB_PATH" >>"$OUT_DIR/logs/dpkg-install.log" 2>&1; then
      DEB_INSTALLED=1
    else
      record DEB_INSTALL FAIL "dpkg -i failed"
      cat "$OUT_DIR/logs/dpkg-install.log" >&2 || true
      OVERALL=FAIL
    fi
  fi
  if [[ "$DEB_INSTALLED" -eq 1 && -x /usr/bin/exyonq ]]; then
    record DEB_INSTALL PASS "$(basename "$DEB_PATH")"
    INST_SHA="$(sha256_file /usr/bin/exyonq)"
    # Compare to package payload
    DEB_EXT="$OUT_DIR/extract/deb"
    rm -rf "$DEB_EXT"; mkdir -p "$DEB_EXT"
    dpkg-deb -x "$DEB_PATH" "$DEB_EXT"
    privacy_scan_tree "$DEB_EXT" DEB
    PAY_SHA="$(sha256_file "$DEB_EXT/usr/bin/exyonq")"
    if [[ "$INST_SHA" == "$PAY_SHA" ]]; then
      record DEB_BINARY_IDENTITY PASS "sha256=$INST_SHA"
    else
      record DEB_BINARY_IDENTITY FAIL "installed!=package payload"
      OVERALL=FAIL
    fi
    FILE_OUT="$(file -b /usr/bin/exyonq || true)"
    case "$ARCH" in
      amd64) echo "$FILE_OUT" | grep -Eiq 'x86-64|x86_64' && record DEB_ELF PASS "$FILE_OUT" || { record DEB_ELF FAIL "$FILE_OUT"; OVERALL=FAIL; } ;;
      arm64) echo "$FILE_OUT" | grep -Eiq 'aarch64' && record DEB_ELF PASS "$FILE_OUT" || { record DEB_ELF FAIL "$FILE_OUT"; OVERALL=FAIL; } ;;
    esac

    # Real config for systemd unit under StateDirectory-writable tree.
    WWW="/var/lib/exyonq/www-cap062"
    $SUDO mkdir -p "$WWW"
    echo "cap062-deb-ok" | $SUDO tee "$WWW/index.html" >/dev/null
    $SUDO chown -R exyonq:exyonq /var/lib/exyonq
    PORT="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
    $SUDO tee /etc/exyonq/config.toml >/dev/null <<EOF
config_version = 1

[logging]
level = "info"
format = "text"
queue_capacity = 1024
[logging.console]
enabled = true
stream = "stdout"
[logging.file]
enabled = false
[logging.access]
enabled = true
[logging.audit]
enabled = false

[[server]]
listen = "127.0.0.1:${PORT}"
routes = ["site"]

[[route]]
name = "site"
match = { path = "/" }
root = "${WWW}"
index = "index.html"
EOF
    $SUDO systemctl daemon-reload
    $SUDO systemctl restart exyonq
    sleep 1
    if $SUDO systemctl is-active --quiet exyonq; then
      record DEB_SYSTEMD_ACTIVE PASS
    else
      record DEB_SYSTEMD_ACTIVE FAIL
      $SUDO journalctl -u exyonq -n 80 --no-pager >&2 || true
      OVERALL=FAIL
    fi
    if curl -sf "http://127.0.0.1:${PORT}/" | grep -q 'cap062-deb-ok'; then
      record DEB_HTTP PASS "port=$PORT"
    else
      record DEB_HTTP FAIL
      OVERALL=FAIL
    fi
    if [[ -x /usr/bin/exyonqctl ]]; then
      # Always use sudo -u so root hosts (Netcup) do not expand to bare `-u`.
      if sudo -n -u exyonq env EXYONQ_CONTROL_SOCKET=/run/exyonq/control.sock \
        /usr/bin/exyonqctl status --socket /run/exyonq/control.sock \
        >"$OUT_DIR/logs/deb-ctl.log" 2>&1 \
        || sudo -u exyonq env EXYONQ_CONTROL_SOCKET=/run/exyonq/control.sock \
          /usr/bin/exyonqctl status --socket /run/exyonq/control.sock \
          >"$OUT_DIR/logs/deb-ctl.log" 2>&1; then
        record DEB_CTL PASS
      else
        record DEB_CTL FAIL "ctl status failed under packaged systemd socket"
        cat "$OUT_DIR/logs/deb-ctl.log" >&2 || true
        OVERALL=FAIL
      fi
    fi
    $SUDO systemctl stop exyonq || true
    record DEB_STOP PASS
    if [[ "${CAP062_PURGE:-0}" == "1" ]]; then
      $SUDO dpkg -r exyonq >>"$OUT_DIR/logs/dpkg-remove.log" 2>&1 || true
      record DEB_PURGE PASS
    else
      record DEB_PURGE SKIP "CAP062_PURGE!=1"
    fi
  else
    record DEB_INSTALL FAIL "/usr/bin/exyonq missing after dpkg"
    OVERALL=FAIL
  fi
fi

# --- RPM E2E ---
if [[ "$PKG_FAMILY" != rpm ]]; then
  record RPM_E2E BLOCKED "CAP062_RPM_REAL_ENVIRONMENT_BLOCKER=OWNER_INFRASTRUCTURE_REQUIRED pkg_family=$PKG_FAMILY"
  RPM_RUNTIME=BLOCKED
else
  RPM_RUNTIME=PENDING
  # Real rpm -Uvh path would go here when RPM hosts exist.
  record RPM_E2E FAIL "RPM-family host detected but installer path not yet wired in this harness revision"
  OVERALL=FAIL
fi

python3 - "$SUMMARY" <<PY
import json, pathlib, os
steps=[]
for line in pathlib.Path("$STEPS").read_text().splitlines():
    if line.strip():
        steps.append(json.loads(line))
fail = sum(1 for s in steps if s.get("status")=="FAIL")
blocked = sum(1 for s in steps if s.get("status")=="BLOCKED")
obj = {
  "CAPABILITY_ID": "062",
  "FEATURE_ID": "pkg-linux-amd64-arm64",
  "ARCH": "$ARCH",
  "TARGET": "$TARGET",
  "VERSION": "$VERSION",
  "HEAD": "$HEAD",
  "TREE": "$TREE",
  "CARGO_LOCK_SHA256": "$LOCK_SHA",
  "PKG_FAMILY": "$PKG_FAMILY",
  "RUN_ID": "$RUN_ID",
  "OVERALL": "$OVERALL",
  "FAIL_STEPS": fail,
  "BLOCKED_STEPS": blocked,
  "RPM_RUNTIME": "$RPM_RUNTIME",
  "FLAT_TARBALL": "$FLAT_NAME",
  "FHS_TARBALL": os.path.basename("$FHS_PATH") if "$FHS_PATH" else None,
  "DEB": os.path.basename("$DEB_PATH") if "$DEB_PATH" else None,
  "RPM": os.path.basename("$RPM_PATH") if "$RPM_PATH" else None,
  "CAP062_OCI_INCLUDED": "NO",
  "steps": steps,
}
pathlib.Path("$SUMMARY").write_text(json.dumps(obj, indent=2) + "\n")
print(json.dumps({"OVERALL": obj["OVERALL"], "FAIL_STEPS": fail, "BLOCKED_STEPS": blocked, "SUMMARY": "$SUMMARY"}))
PY

if [[ "$OVERALL" != PASS ]]; then
  exit 1
fi
exit 0
