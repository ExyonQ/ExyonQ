#!/usr/bin/env bash
# EXYONQ-PUBLICATION-PRIVACY-GATE
# Rule 135: public material (release body, changelog, audit, docs copied to the
# staging tree) must never leak lab paths, internal host names or campaign
# codes. This gate is the mechanical check behind that rule.
#
# Usage:
#   publication-privacy-gate.sh --tree DIR [--allow FILE]
#   publication-privacy-gate.sh --selftest
#
# Exit: 0 clean, 1 findings.
set -euo pipefail
LC_ALL=C
export LC_ALL
set +x

RULE_ID=EXYONQ-PUBLICATION-PRIVACY-GATE
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TREE=""
ALLOW=""
SELFTEST=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tree) TREE="${2:?}"; shift 2 ;;
    --allow) ALLOW="${2:?}"; shift 2 ;;
    --selftest) SELFTEST=1; shift ;;
    -h|--help) sed -n '2,13p' "$0"; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; exit 2 ;;
  esac
done

scan_tree() {
  local tree="$1" allow="${2:-}"
  python3 - "$tree" "$allow" <<'PY'
import re, sys, pathlib

tree = pathlib.Path(sys.argv[1])
allow_file = sys.argv[2]

RULES = {
    "LAB_PATH": re.compile(r"/Volumes/[A-Za-z]|/Users/[A-Za-z]"),
    "INTERNAL_HOST": re.compile(r"\b(netcup-bench|oracle-quasar|servidor-nx1)\b"),
    "CAMPAIGN_CODE": re.compile(r"\b(rule[ _-]?134|adelanto)\b", re.IGNORECASE),
    "PREP_PLACEHOLDER": re.compile(r"PUBLICATION\s*=\s*PENDING|_pending_publish_commit_|finalize before tag"),
}

allow = set()
if allow_file:
    for line in pathlib.Path(allow_file).read_text(errors="ignore").splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            allow.add(line)

SKIP_DIRS = {".git", "target", "node_modules", ".exyonq-local", "__pycache__", ".venv"}
SKIP_SUFFIXES = {".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".pdf", ".zip", ".gz", ".woff", ".woff2", ".ttf"}

findings = 0
for path in sorted(tree.rglob("*")):
    if not path.is_file():
        continue
    rel = str(path.relative_to(tree)).replace("\\", "/")
    if any(part in SKIP_DIRS for part in rel.split("/")):
        continue
    if path.suffix.lower() in SKIP_SUFFIXES:
        continue
    if rel in allow:
        continue
    try:
        text = path.read_text(errors="ignore")
    except OSError:
        continue
    for lineno, line in enumerate(text.splitlines(), 1):
        for rule, pattern in RULES.items():
            if pattern.search(line):
                print(f"FINDING {rule} {rel}:{lineno}")
                findings += 1
print(f"FINDINGS={findings}")
sys.exit(0 if findings == 0 else 1)
PY
}

selftest() {
  local tmp status=0
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN

  mkdir -p "$tmp/clean/docs"
  printf '# Release notes\n\nNothing private here.\n' >"$tmp/clean/docs/notes.md"
  if ! scan_tree "$tmp/clean" >/dev/null; then
    echo "SELFTEST_FAIL clean_tree_must_pass"; status=1
  else
    echo "SELFTEST_PASS clean_tree_must_pass"
  fi

  for case in 'LAB_PATH:<workspace>/lab/x.md' \
              'INTERNAL_HOST:deployed on netcup-bench today' \
              'CAMPAIGN_CODE:see rule 134 for the gate' \
              'PREP_PLACEHOLDER:PUBLICATION = PENDING'; do
    rule="${case%%:*}"; text="${case#*:}"
    mkdir -p "$tmp/dirty-$rule"
    printf '%s\n' "$text" >"$tmp/dirty-$rule/x.md"
    if scan_tree "$tmp/dirty-$rule" >/dev/null; then
      echo "SELFTEST_FAIL ${rule}_must_fail"; status=1
    else
      echo "SELFTEST_PASS ${rule}_must_fail"
    fi
  done

  printf '# allowlisted\nx.md\n' >"$tmp/allow.txt"
  mkdir -p "$tmp/allowed"
  printf 'deployed on netcup-bench\n' >"$tmp/allowed/x.md"
  if ! scan_tree "$tmp/allowed" "$tmp/allow.txt" >/dev/null; then
    echo "SELFTEST_FAIL allowlist_must_pass"; status=1
  else
    echo "SELFTEST_PASS allowlist_must_pass"
  fi

  echo "PUBLICATION_PRIVACY_GATE_SELFTEST=$([[ $status -eq 0 ]] && echo PASS || echo FAIL)"
  return "$status"
}

if [[ "$SELFTEST" == "1" ]]; then
  selftest
  exit $?
fi

# Default target: the material that actually gets published with a release.
# The full tree is intentionally *not* the default: the lab tree (and the
# historical part of the public tree) legitimately mentions internal hosts, and
# scanning it drowns the signal. Pass --tree for a full audit.
if [[ -z "$TREE" ]]; then
  TMP_TREE="$(mktemp -d)"
  trap 'rm -rf "$TMP_TREE"' EXIT
  mkdir -p "$TMP_TREE/docs"
  for f in docs/release CHANGELOG.md SECURITY.md README.md CODE_OF_CONDUCT.md; do
    [[ -e "$ROOT/$f" ]] && cp -R "$ROOT/$f" "$TMP_TREE/$f" 2>/dev/null || true
  done
  TREE="$TMP_TREE"
fi
out="$(scan_tree "$TREE" "$ALLOW" || true)"
printf '%s\n' "$out"
findings="$(printf '%s\n' "$out" | tail -1 | cut -d= -f2)"
echo "RULE_ID=$RULE_ID"
echo "PUBLICATION_PRIVACY_FINDINGS=$findings"
if [[ "$findings" != "0" ]]; then
  echo "PUBLICATION_PRIVACY_GATE=FAIL"
  exit 1
fi
echo "PUBLICATION_PRIVACY_GATE=PASS"
