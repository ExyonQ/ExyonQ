#!/usr/bin/env bash
# Shell portability: tracked shell scripts must run on the bash 3.2 that macOS
# ships, because it is the interactive shell of this project's development host.
#
# Two tracked scripts used bash-4-only constructs and aborted there before doing
# any work — `scripts/fuzz/p15-ws2-run.sh` (`declare -A` for the per-target
# budget, so the whole fuzz matrix died with `declare: -A: invalid option`) and
# `scripts/release/p15-ws7-final-reconciliation.sh` (same construct for the
# lock→evidence directory map). Nothing caught it because the fuzz runner is
# invoked by hand and there was no lint over the shell surface. This gate is
# that lint, so the class cannot come back unnoticed.
#
# It scans the tracked `*.sh`/`*.bash` set (not untracked local copies), skips
# comment-only mentions and prints `file:line: source` for every finding.
#
# Usage:
#   bash scripts/gates/portable-shell-gate.sh
#   bash scripts/gates/portable-shell-gate.sh --repo DIR
#   bash scripts/gates/portable-shell-gate.sh --selftest
set -uo pipefail
LC_ALL=C
export LC_ALL

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

usage() {
  cat <<'EOF'
Usage:
  portable-shell-gate.sh [--repo DIR]
  portable-shell-gate.sh --selftest

Fails when a tracked *.sh/*.bash file uses a bash >= 4 only construct:
  declare -A / declare -n / local -n / mapfile / readarray / ${var,,} / ${var^^}
Comment lines and trailing `# ...` comments are ignored.
EOF
}

die_usage() {
  echo "ERROR: $1" >&2
  usage >&2
  exit 2
}

# scan_repo <repo> -> prints findings and the count on stderr as FINDINGS=<n>
scan_repo() {
  local repo="$1"
  local files=0 findings=0
  local f
  while IFS= read -r f; do
    [[ -n "$f" ]] || continue
    files=$((files + 1))
    local hits
    hits="$(awk -v F="$f" '
      {
        line = $0
        code = line
        sub(/#.*/, "", code)
        trimmed = line
        sub(/^[ \t]+/, "", trimmed)
        if (trimmed ~ /^#/) next
        if (code ~ /declare[ \t]+-[A-Za-z]*[An]/ ||
            code ~ /(local|declare)[ \t]+-n/ ||
            code ~ /(^|[^A-Za-z0-9_])(mapfile|readarray)([^A-Za-z0-9_]|$)/ ||
            code ~ /\$\{[A-Za-z_][A-Za-z0-9_]*(,,?|\^\^?)\}/) {
          print F ":" NR ": " trimmed
        }
      }' "$repo/$f")"
    if [[ -n "$hits" ]]; then
      printf '%s\n' "$hits"
      findings=$((findings + $(printf '%s\n' "$hits" | wc -l | tr -d ' ')))
    fi
  done < <(git -C "$repo" ls-files '*.sh' '*.bash')
  echo "PORTABLE_SHELL_FILES=$files" >&2
  echo "PORTABLE_SHELL_FINDINGS=$findings" >&2
  [[ "$findings" -eq 0 ]]
}

selftest() {
  local tmp failures=0 repo
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/portable-shell-gate.XXXXXX")" || {
    echo "RESULT=FAIL PORTABLE_SHELL_GATE_SELFTEST (mktemp)"
    return 2
  }
  SELFTEST_TMP="$tmp"
  trap 'rm -rf "${SELFTEST_TMP:-}"' EXIT
  repo="$tmp/repo"
  mkdir -p "$repo/scripts"
  git -C "$repo" init -q
  git -C "$repo" config user.name "selftest"
  git -C "$repo" config user.email "selftest@example.invalid"

  cat >"$repo/scripts/ok.sh" <<'EOF'
#!/usr/bin/env bash
# Portable: plain arrays, case lookups, and a comment mentioning declare -A.
lookup() {
  case "$1" in
    a|b) echo one ;;
    *) return 1 ;;
  esac
}
lookup a >/dev/null
list=(x y)
printf '%s\n' "${list[0]}"
EOF
  cat >"$repo/scripts/bad.sh" <<'EOF'
#!/usr/bin/env bash
declare -A MAP=()
EOF
  git -C "$repo" add -A

  local out rc
  out="$(bash "${BASH_SOURCE[0]}" --repo "$repo" 2>&1)"
  rc=$?
  if [[ "$rc" -eq 1 ]] && printf '%s\n' "$out" | grep -q "scripts/bad.sh:2"; then
    echo "PASS selftest detects declare -A"
  else
    echo "FAIL selftest detects declare -A (rc=$rc)"
    printf '%s\n' "$out" | sed 's/^/  /'
    failures=$((failures + 1))
  fi

  git -C "$repo" rm -q --cached scripts/bad.sh
  rm -f "$repo/scripts/bad.sh"
  out="$(bash "${BASH_SOURCE[0]}" --repo "$repo" 2>&1)"
  rc=$?
  if [[ "$rc" -eq 0 ]] && printf '%s\n' "$out" | grep -q "RESULT=PASS PORTABLE_SHELL_GATE"; then
    echo "PASS selftest portable tree passes and comment mention is ignored"
  else
    echo "FAIL selftest portable tree (rc=$rc)"
    printf '%s\n' "$out" | sed 's/^/  /'
    failures=$((failures + 1))
  fi

  cat >"$repo/scripts/mapfile.sh" <<'EOF'
#!/usr/bin/env bash
mapfile -t lines < <(printf 'a\n')
local_test() { local -n ref=$1; printf '%s\n' "$ref"; }
EOF
  git -C "$repo" add -A
  out="$(bash "${BASH_SOURCE[0]}" --repo "$repo" 2>&1)"
  rc=$?
  if [[ "$rc" -eq 1 ]] &&
    printf '%s\n' "$out" | grep -q "scripts/mapfile.sh:2" &&
    printf '%s\n' "$out" | grep -q "scripts/mapfile.sh:3"; then
    echo "PASS selftest detects mapfile and local -n"
  else
    echo "FAIL selftest detects mapfile/local -n (rc=$rc)"
    printf '%s\n' "$out" | sed 's/^/  /'
    failures=$((failures + 1))
  fi

  if [[ "$failures" -ne 0 ]]; then
    echo "RESULT=FAIL PORTABLE_SHELL_GATE_SELFTEST failures=$failures"
    return 1
  fi
  echo "RESULT=PASS PORTABLE_SHELL_GATE_SELFTEST"
  return 0
}

REPO="$ROOT"
SELFTEST=0
SELFTEST_TMP=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo) [[ $# -ge 2 ]] || die_usage "--repo needs a value"; REPO="$2"; shift 2 ;;
    --selftest) SELFTEST=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die_usage "unknown argument: $1" ;;
  esac
done

if [[ "$SELFTEST" == "1" ]]; then
  selftest
  exit $?
fi

git -C "$REPO" rev-parse --git-dir >/dev/null 2>&1 || {
  echo "FAIL not a git repository: $REPO"
  echo "RESULT=FAIL PORTABLE_SHELL_GATE"
  exit 2
}

if scan_repo "$REPO"; then
  echo "RESULT=PASS PORTABLE_SHELL_GATE"
  exit 0
fi
echo "RESULT=FAIL PORTABLE_SHELL_GATE (bash >= 4 constructs in tracked shell scripts)"
exit 1
