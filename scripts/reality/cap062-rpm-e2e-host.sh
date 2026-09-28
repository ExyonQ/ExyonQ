#!/usr/bin/env bash
# Cap062 — RPM-family terminal E2E (prepared; NOT executable as PASS without real authorities).
#
# FORBIDDEN as terminal proof:
#   Ubuntu+rpm tooling, rpm2cpio-only, alien, fake RPM DB, chroot cosplay,
#   container-as-systemd-authority, mocked systemd, metadata-only, ELF-only, nfpm exit 0.
#
# Until REAL_RPM_{AMD64,ARM64}_AUTHORITY are owner-provided, this harness MUST exit
# with CAP062_RPM_RUNTIME = NOT_EXECUTED / BLOCKED — never PASS_REAL_PRODUCTION.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

usage() {
  cat <<'EOF'
Usage:
  cap062-rpm-e2e-host.sh --rpm PATH --arch x86_64|aarch64 [--out-dir DIR]

Requires a real RPM-family host (Rocky/Alma/Fedora/RHEL-compatible) matching --arch.
Records host inventory, then runs native dnf/yum/rpm install → systemd → HTTP → ctl → stop.
EOF
}

RPM_PATH=""
ARCH_EXPECT=""
OUT_DIR=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --rpm) RPM_PATH="$2"; shift 2 ;;
    --arch) ARCH_EXPECT="$2"; shift 2 ;;
    --out-dir) OUT_DIR="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown arg $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$RPM_PATH" && -f "$RPM_PATH" && -n "$ARCH_EXPECT" ]] || { usage >&2; exit 2; }
[[ -n "$OUT_DIR" ]] || OUT_DIR="$ROOT/.exyonq-local/tmp/phase1-cap062-rpm-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$OUT_DIR"/{logs,evidence}
STEPS="$OUT_DIR/steps.jsonl"
: >"$STEPS"

record() {
  printf '{"step":"%s","status":"%s","detail":%s}\n' \
    "$1" "$2" "$(python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "${3:-}")" >>"$STEPS"
  echo "[cap062-rpm] $1=$2 ${3:-}" >&2
}

# --- Authority gate (fail closed) ---
OS_ID="$(. /etc/os-release 2>/dev/null; echo "${ID:-unknown}")"
OS_LIKE="$(. /etc/os-release 2>/dev/null; echo "${ID_LIKE:-}")"
OS_VERSION="$(. /etc/os-release 2>/dev/null; echo "${VERSION_ID:-unknown}")"
UNAME_M="$(uname -m)"
KERNEL="$(uname -r)"

is_rpm_family=0
case "$OS_ID" in
  rocky|almalinux|fedora|rhel|centos) is_rpm_family=1 ;;
esac
if echo "$OS_LIKE" | grep -Eiq 'rhel|fedora|centos'; then
  is_rpm_family=1
fi
# Explicit reject Debian/Ubuntu even if rpm CLI exists.
if [[ -f /etc/debian_version ]] || [[ "$OS_ID" == debian || "$OS_ID" == ubuntu ]]; then
  is_rpm_family=0
fi

{
  echo "HOST_ROLE=RPM_${ARCH_EXPECT}_AUTHORITY_CANDIDATE"
  echo "OS_NAME=$OS_ID"
  echo "OS_VERSION=$OS_VERSION"
  echo "KERNEL=$KERNEL"
  echo "ARCH=$UNAME_M"
  echo "CPU_ARCH_FROM_UNAME=$UNAME_M"
  echo "RPM_VERSION=$(rpm --version 2>/dev/null || echo MISSING)"
  echo "PACKAGE_MANAGER=$(command -v dnf >/dev/null && echo dnf || command -v yum >/dev/null && echo yum || echo rpm)"
  echo "SYSTEMD_VERSION=$(systemctl --version 2>/dev/null | head -n1 || echo MISSING)"
  echo "GLIBC_VERSION=$(ldd --version 2>/dev/null | head -n1 || echo MISSING)"
  echo "SSH_REACHABILITY=LOCAL_OR_REMOTE_SESSION"
  echo "AVAILABLE_DISK=$(df -h / | awk 'NR==2{print $4}')"
  echo "AVAILABLE_RAM=$(awk '/MemAvailable/{printf "%.0fMiB",$2/1024}' /proc/meminfo 2>/dev/null || echo UNKNOWN)"
} >"$OUT_DIR/evidence/host_inventory.txt"

if [[ "$is_rpm_family" -ne 1 ]]; then
  record AUTHORITY_GATE BLOCKED "not RPM-family OS (id=$OS_ID); CAP062_RPM_RUNTIME=NOT_EXECUTED"
  cat >"$OUT_DIR/summary.json" <<EOF
{"CAPABILITY_ID":"062","SURFACE":"RPM","ARCH_EXPECT":"$ARCH_EXPECT","OVERALL":"BLOCKED",
 "REASON":"OWNER_INFRASTRUCTURE_NOT_CURRENTLY_AVAILABLE_OR_WRONG_OS",
 "RPM_PASS_REAL_PRODUCTION":"NO","HOST_INVENTORY":"$OUT_DIR/evidence/host_inventory.txt"}
EOF
  echo "CAP062_RPM_E2E=BLOCKED — do not interpret as PASS" >&2
  exit 3
fi

case "$ARCH_EXPECT" in
  x86_64) [[ "$UNAME_M" == x86_64 ]] || { record ARCH_MATCH FAIL "host=$UNAME_M expect=x86_64"; exit 1; } ;;
  aarch64) [[ "$UNAME_M" == aarch64 ]] || { record ARCH_MATCH FAIL "host=$UNAME_M expect=aarch64"; exit 1; } ;;
  *) echo "ERROR: --arch must be x86_64 or aarch64" >&2; exit 2 ;;
esac
record ARCH_MATCH PASS "uname=$UNAME_M"

# --- Artifact bind ---
RPM_PACKAGE_FILE="$(cd "$(dirname "$RPM_PATH")" && pwd)/$(basename "$RPM_PATH")"
RPM_PACKAGE_SHA256="$(sha256sum "$RPM_PACKAGE_FILE" | awk '{print $1}')"
record CHECKSUM PASS "sha256=$RPM_PACKAGE_SHA256"

RPM_METADATA_NAME="$(rpm -qp --qf '%{NAME}' "$RPM_PACKAGE_FILE")"
RPM_METADATA_VERSION="$(rpm -qp --qf '%{VERSION}-%{RELEASE}' "$RPM_PACKAGE_FILE")"
RPM_METADATA_ARCH="$(rpm -qp --qf '%{ARCH}' "$RPM_PACKAGE_FILE")"
record RPM_METADATA PASS "name=$RPM_METADATA_NAME version=$RPM_METADATA_VERSION arch=$RPM_METADATA_ARCH"

case "$ARCH_EXPECT" in
  x86_64) echo "$RPM_METADATA_ARCH" | grep -Eiq 'x86_64' || { record PKG_ARCH FAIL "$RPM_METADATA_ARCH"; exit 1; } ;;
  aarch64) echo "$RPM_METADATA_ARCH" | grep -Eiq 'aarch64' || { record PKG_ARCH FAIL "$RPM_METADATA_ARCH"; exit 1; } ;;
esac

# Payload SHA (inspection aid; not install proof)
TMP_PAY="$(mktemp -d)"
set +e
(cd "$TMP_PAY" && rpm2cpio "$RPM_PACKAGE_FILE" | cpio -idm --no-absolute-filenames >/dev/null 2>&1)
set -e
[[ -f "$TMP_PAY/usr/bin/exyonq" ]] || { record RPM_PAYLOAD FAIL "missing usr/bin/exyonq"; rm -rf "$TMP_PAY"; exit 1; }
RPM_PAYLOAD_EXYONQ_SHA256="$(sha256sum "$TMP_PAY/usr/bin/exyonq" | awk '{print $1}')"
FILE_OUT="$(file -b "$TMP_PAY/usr/bin/exyonq")"
case "$ARCH_EXPECT" in
  x86_64) echo "$FILE_OUT" | grep -Eiq 'x86-64|x86_64' || { record ELF FAIL "$FILE_OUT"; exit 1; } ;;
  aarch64) echo "$FILE_OUT" | grep -Eiq 'aarch64' || { record ELF FAIL "$FILE_OUT"; exit 1; } ;;
esac
record ELF PASS "$FILE_OUT"
rm -rf "$TMP_PAY"

# --- Native install ---
SUDO=""
[[ "$(id -u)" -eq 0 ]] || SUDO="sudo -n"
if command -v dnf >/dev/null 2>&1; then
  $SUDO dnf -y install "$RPM_PACKAGE_FILE" >"$OUT_DIR/logs/dnf-install.log" 2>&1
elif command -v yum >/dev/null 2>&1; then
  $SUDO yum -y localinstall "$RPM_PACKAGE_FILE" >"$OUT_DIR/logs/yum-install.log" 2>&1
else
  $SUDO rpm -Uvh "$RPM_PACKAGE_FILE" >"$OUT_DIR/logs/rpm-install.log" 2>&1
fi
record RPM_INSTALL PASS "native package manager"

INSTALLED_PACKAGE_NAME="$(rpm -q --qf '%{NAME}' exyonq)"
INSTALLED_PACKAGE_VERSION="$(rpm -q --qf '%{VERSION}-%{RELEASE}' exyonq)"
INSTALLED_PACKAGE_ARCH="$(rpm -q --qf '%{ARCH}' exyonq)"
INSTALLED_EXYONQ_PATH="/usr/bin/exyonq"
INSTALLED_EXYONQ_SHA256="$(sha256sum "$INSTALLED_EXYONQ_PATH" | awk '{print $1}')"
[[ "$INSTALLED_EXYONQ_SHA256" == "$RPM_PAYLOAD_EXYONQ_SHA256" ]] || {
  record BINARY_IDENTITY FAIL "installed!=payload"
  exit 1
}
record BINARY_IDENTITY PASS "sha256=$INSTALLED_EXYONQ_SHA256"

# Layout
for p in /usr/bin/exyonq /usr/bin/exyonqctl /etc/exyonq /usr/lib/systemd/system/exyonq.service; do
  [[ -e "$p" ]] || { record LAYOUT FAIL "missing $p"; exit 1; }
done
record FHS_LAYOUT PASS

# systemd + HTTP
WWW="/var/lib/exyonq/www-cap062-rpm"
$SUDO mkdir -p "$WWW"
echo "cap062-rpm-ok" | $SUDO tee "$WWW/index.html" >/dev/null
$SUDO chown -R exyonq:exyonq /var/lib/exyonq 2>/dev/null || true
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
$SUDO systemctl is-active --quiet exyonq || { record SYSTEMD_ACTIVE FAIL; $SUDO journalctl -u exyonq -n 80 --no-pager; exit 1; }
record SYSTEMD_ACTIVE PASS
curl -sf "http://127.0.0.1:${PORT}/" | grep -q 'cap062-rpm-ok' || { record HTTP FAIL; exit 1; }
record HTTP PASS "port=$PORT"
if sudo -n -u exyonq /usr/bin/exyonqctl status --socket /run/exyonq/control.sock \
  >"$OUT_DIR/logs/ctl.log" 2>&1 \
  || sudo -u exyonq /usr/bin/exyonqctl status --socket /run/exyonq/control.sock \
  >"$OUT_DIR/logs/ctl.log" 2>&1; then
  record CTL PASS
else
  record CTL FAIL; cat "$OUT_DIR/logs/ctl.log" >&2; exit 1
fi
$SUDO systemctl stop exyonq
record STOP PASS

# Evidence bind fields
cat >"$OUT_DIR/evidence/rpm_identity.txt" <<EOF
RPM_PACKAGE_FILE=$RPM_PACKAGE_FILE
RPM_PACKAGE_SHA256=$RPM_PACKAGE_SHA256
RPM_METADATA_NAME=$RPM_METADATA_NAME
RPM_METADATA_VERSION=$RPM_METADATA_VERSION
RPM_METADATA_ARCH=$RPM_METADATA_ARCH
INSTALLED_PACKAGE_NAME=$INSTALLED_PACKAGE_NAME
INSTALLED_PACKAGE_VERSION=$INSTALLED_PACKAGE_VERSION
INSTALLED_PACKAGE_ARCH=$INSTALLED_PACKAGE_ARCH
INSTALLED_EXYONQ_PATH=$INSTALLED_EXYONQ_PATH
INSTALLED_EXYONQ_SHA256=$INSTALLED_EXYONQ_SHA256
RPM_PAYLOAD_EXYONQ_SHA256=$RPM_PAYLOAD_EXYONQ_SHA256
EOF

python3 - "$OUT_DIR/summary.json" <<PY
import json, pathlib
steps=[json.loads(l) for l in pathlib.Path("$STEPS").read_text().splitlines() if l.strip()]
fail=sum(1 for s in steps if s["status"]=="FAIL")
obj={
  "CAPABILITY_ID":"062","SURFACE":"RPM","ARCH":"$ARCH_EXPECT",
  "OVERALL": "FAIL" if fail else "PASS",
  "RPM_PASS_REAL_PRODUCTION": "NO" if fail else "YES_ONLY_IF_AUTHORITY_WAS_REAL_RPM_FAMILY",
  "steps": steps,
}
pathlib.Path("$OUT_DIR/summary.json").write_text(json.dumps(obj, indent=2)+"\n")
print(json.dumps({"OVERALL": obj["OVERALL"], "OUT": "$OUT_DIR"}))
PY

[[ "$(python3 -c 'import json;print(json.load(open("'"$OUT_DIR"'/summary.json"))["OVERALL"])')" == PASS ]]
