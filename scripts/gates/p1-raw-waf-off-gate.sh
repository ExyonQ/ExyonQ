#!/usr/bin/env bash
# Fail-closed guard: canonical P1 RAW ExyonQ config must resolve to WAF DISABLED.
#
# Verifies effective IR semantics (same resolution rules as product defaults),
# not merely that the substring "[waf]" appears.
#
# PRODUCT_MUTATION = NO — harness/config gate only.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CFG="${P1_RAW_EXYONQ_CONFIG:-$ROOT/benchmarks/configs/exyonq/bench.toml}"
SELFTEST=0
TREE_MODE=0

usage() {
  cat <<'EOF'
Usage:
  scripts/gates/p1-raw-waf-off-gate.sh [--tree .] [--config PATH]
  scripts/gates/p1-raw-waf-off-gate.sh --selftest
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tree)
      ROOT="$(cd "$2" && pwd)"
      CFG="${P1_RAW_EXYONQ_CONFIG:-$ROOT/benchmarks/configs/exyonq/bench.toml}"
      TREE_MODE=1
      shift 2
      ;;
    --config)
      CFG="$2"
      shift 2
      ;;
    --selftest)
      SELFTEST=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

check_cfg() {
  local path="$1"
  local expect_pass="$2" # YES|NO
  python3 - "$path" "$expect_pass" <<'PY'
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # py<3.11
    import tomli as tomllib  # type: ignore

path = Path(sys.argv[1])
expect_pass = sys.argv[2] == "YES"
raw = path.read_text(encoding="utf-8")
data = tomllib.loads(raw)

# Product IR defaults (config/ir/src/waf.rs) — must stay mirrored here for the gate.
DEFAULT_ENABLED = True
DEFAULT_MODE = "monitor"

if "waf" not in data:
    enabled = DEFAULT_ENABLED
    mode = DEFAULT_MODE
    section_present = False
else:
    section_present = True
    waf = data["waf"]
    if not isinstance(waf, dict):
        print("P1_RAW_WAF_GATE=FAIL reason=waf_not_table")
        sys.exit(1)
    enabled = bool(waf.get("enabled", DEFAULT_ENABLED))
    mode = str(waf.get("mode", DEFAULT_MODE)).strip().lower()

# Effective signature activity (mirrors cli/exyonq/src/waf_install.rs):
# signature_active = enabled && mode != disabled
effective_active = enabled and mode != "disabled"
effective_mode = "DISABLED" if not effective_active else mode.upper()

print(f"CONFIG_PATH={path}")
print(f"WAF_SECTION_PRESENT={'YES' if section_present else 'NO'}")
print(f"WAF_ENABLED_PARSED={str(enabled).upper()}")
print(f"WAF_MODE_PARSED={mode}")
print(f"P1_RAW_WAF_EFFECTIVE_MODE={effective_mode}")
print(f"P1_RAW_WAF_SIGNATURE_ACTIVE={'YES' if effective_active else 'NO'}")

ok = section_present and (not enabled) and mode == "disabled" and (not effective_active)
if expect_pass:
    if not ok:
        print("P1_RAW_WAF_GATE=FAIL")
        print("REQUIRED: [waf] enabled=false mode=\"disabled\" → effective DISABLED")
        sys.exit(1)
    print("P1_RAW_WAF_GATE=PASS")
else:
    if ok:
        print("P1_RAW_WAF_GATE_SELFTEST=FAIL unexpected_pass")
        sys.exit(1)
    print("P1_RAW_WAF_GATE_SELFTEST_NEGATIVE=PASS")
PY
}

if [[ "$SELFTEST" -eq 1 ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  cat >"$tmp/absent.toml" <<'EOF'
config_version = 1
[[server]]
listen = "0.0.0.0:8080"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/site" }
root = "/tmp"
EOF
  cat >"$tmp/ok.toml" <<'EOF'
config_version = 1
[[server]]
listen = "0.0.0.0:8080"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/site" }
root = "/tmp"
[waf]
enabled = false
mode = "disabled"
EOF
  check_cfg "$tmp/absent.toml" NO
  check_cfg "$tmp/ok.toml" YES
  echo "P1_RAW_WAF_OFF_GATE_SELFTEST=PASS"
  exit 0
fi

if [[ ! -f "$CFG" ]]; then
  echo "P1_RAW_WAF_GATE=FAIL reason=missing_config path=$CFG" >&2
  exit 1
fi

check_cfg "$CFG" YES
if [[ "$TREE_MODE" -eq 1 ]]; then
  echo "TREE=$ROOT"
fi
exit 0
