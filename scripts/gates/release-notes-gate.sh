#!/usr/bin/env bash
# Public release notes guard. The GitHub Release body is the short, user-facing
# note in docs/release/notes/v<version>.md — never the audit record in
# docs/release/v<version>.md (0.9.3 published 612 lines / 51 KB as its release
# body). release.yml selects that file and calls this gate, so a missing,
# oversized or audit-shaped body fails before the Release is created.
#
# Usage:
#   bash scripts/gates/release-notes-gate.sh --version 0.9.4
#   bash scripts/gates/release-notes-gate.sh --version 0.9.4 --notes path.md
#   bash scripts/gates/release-notes-gate.sh --version 0.9.3 \
#     --notes docs/release/v0.9.3.md --allow-legacy
#   bash scripts/gates/release-notes-gate.sh --selftest
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VERSION=""
NOTES=""
MAX_LINES=80
MAX_BYTES=8192
ALLOW_LEGACY=0

usage() {
  cat <<'EOF'
Usage:
  release-notes-gate.sh --version X.Y.Z [--notes PATH]
                        [--max-lines N] [--max-bytes N] [--allow-legacy]
  release-notes-gate.sh --selftest

Checks the public GitHub Release body:
  - the note exists and is not empty
  - it has at most --max-lines (default 80) and --max-bytes (default 8192)
  - it carries no audit-only markers (seal fingerprints, PUBLICATION_STATUS,
    verification tables, internal checklists)

--allow-legacy skips the size and marker checks: historical tags keep the body
they were published with.
EOF
}

die_usage() {
  echo "error: $*" >&2
  usage >&2
  exit 2
}

is_uint() {
  case "$1" in
    ''|*[!0-9]*) return 1 ;;
    *) return 0 ;;
  esac
}

# check_note <path> <max_lines> <max_bytes> <allow_legacy>
check_note() {
  local path="$1" max_lines="$2" max_bytes="$3" allow_legacy="$4"
  local lines bytes marker

  if [[ ! -f "$path" ]]; then
    echo "RESULT=FAIL RELEASE_NOTES_GATE version=${VERSION} reason=missing path=${path}"
    return 1
  fi
  if [[ ! -s "$path" ]]; then
    echo "RESULT=FAIL RELEASE_NOTES_GATE version=${VERSION} reason=empty path=${path}"
    return 1
  fi

  lines="$(wc -l <"$path" | tr -d '[:space:]')"
  bytes="$(wc -c <"$path" | tr -d '[:space:]')"

  if [[ "$allow_legacy" -eq 0 ]]; then
    for marker in 'WORKTREE_FINGERPRINT' 'PUBLICATION_STATUS =' 'SEAL_INTEGRITY' \
      '## Release checklist' '## Publication record' '## Verification executed' \
      '| Command | Result |'; do
      if grep -Fq -- "$marker" "$path"; then
        echo "RESULT=FAIL RELEASE_NOTES_GATE version=${VERSION} reason=audit_marker marker=[${marker}] path=${path}"
        return 1
      fi
    done
    if [[ "$lines" -gt "$max_lines" ]]; then
      echo "RESULT=FAIL RELEASE_NOTES_GATE version=${VERSION} reason=too_many_lines lines=${lines} max=${max_lines} path=${path}"
      return 1
    fi
    if [[ "$bytes" -gt "$max_bytes" ]]; then
      echo "RESULT=FAIL RELEASE_NOTES_GATE version=${VERSION} reason=too_large bytes=${bytes} max=${max_bytes} path=${path}"
      return 1
    fi
  fi

  echo "RESULT=PASS RELEASE_NOTES_GATE version=${VERSION} path=${path} lines=${lines} bytes=${bytes} legacy=${allow_legacy}"
  return 0
}

selftest() {
  local dir failures=0 want got label path
  dir="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-release-notes.XXXXXX")"

  printf '# 1.2.3\n\nPatch release.\n\n## Changes\n\n- One fix.\n' >"$dir/good.md"
  awk 'BEGIN { for (i = 0; i < 30; i++) print "- line " i }' >"$dir/long.md"
  printf '# 1.2.3\n\nPUBLICATION_STATUS = PUBLISHED\n' >"$dir/marked.md"

  # want, label, path, max_lines, max_bytes, allow_legacy
  run_case() {
    want="$1"
    label="$2"
    path="$3"
    got=0
    check_note "$path" "$4" "$5" "$6" >/dev/null 2>&1 || got=1
    if [[ "$got" -ne "$want" ]]; then
      echo "FAIL: selftest ${label}: expected exit ${want}, got ${got}"
      return 1
    fi
    echo "PASS: selftest ${label}"
    return 0
  }

  run_case 0 "short-note" "$dir/good.md" 80 8192 0 || failures=1
  run_case 1 "too-many-lines" "$dir/long.md" 10 8192 0 || failures=1
  run_case 1 "too-large" "$dir/long.md" 80 100 0 || failures=1
  run_case 1 "audit-marker" "$dir/marked.md" 80 8192 0 || failures=1
  run_case 1 "missing-file" "$dir/absent.md" 80 8192 0 || failures=1
  run_case 0 "legacy-allowed" "$dir/long.md" 10 100 1 || failures=1

  rm -rf "$dir"
  if [[ "$failures" -ne 0 ]]; then
    echo "RESULT=FAIL RELEASE_NOTES_GATE_SELFTEST"
    return 1
  fi
  echo "RESULT=PASS RELEASE_NOTES_GATE_SELFTEST"
  return 0
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      [[ $# -ge 2 ]] || die_usage "--version needs a value"
      VERSION="$2"
      shift 2
      ;;
    --notes)
      [[ $# -ge 2 ]] || die_usage "--notes needs a value"
      NOTES="$2"
      shift 2
      ;;
    --max-lines)
      [[ $# -ge 2 ]] || die_usage "--max-lines needs a value"
      MAX_LINES="$2"
      shift 2
      ;;
    --max-bytes)
      [[ $# -ge 2 ]] || die_usage "--max-bytes needs a value"
      MAX_BYTES="$2"
      shift 2
      ;;
    --allow-legacy) ALLOW_LEGACY=1; shift ;;
    --selftest) selftest; exit $? ;;
    -h|--help) usage; exit 0 ;;
    *) die_usage "unknown argument: $1" ;;
  esac
done

[[ -n "$VERSION" ]] || die_usage "--version is required"
is_uint "$MAX_LINES" || die_usage "--max-lines must be a positive integer"
is_uint "$MAX_BYTES" || die_usage "--max-bytes must be a positive integer"
[[ -n "$NOTES" ]] || NOTES="$ROOT/docs/release/notes/v${VERSION}.md"

check_note "$NOTES" "$MAX_LINES" "$MAX_BYTES" "$ALLOW_LEGACY"
