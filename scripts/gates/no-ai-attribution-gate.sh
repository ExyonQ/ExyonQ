#!/usr/bin/env bash
# Reject AI/bot attribution in commits and commit messages.
#
# Published history must carry no "Generated with ..." footer and no
# Co-Authored-By line naming an assistant (AGENTS.md pre-push checklist). A
# Codebuff trailer on nine commits made codebuff-team appear in the
# repository's Contributors widget; rewriting those messages removed it, and
# this gate keeps the class from coming back.
#
# Usage:
#   no-ai-attribution-gate.sh --range "origin/main..HEAD"
#   no-ai-attribution-gate.sh --range "v0.9.3..HEAD"
#   no-ai-attribution-gate.sh --message-file .git/COMMIT_EDITMSG
#   no-ai-attribution-gate.sh --selftest
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPO="$ROOT"
RANGE=""
MESSAGE_FILE=""
SCANNED=0
DENY='codebuff|freebuff|claude|anthropic|openai|chatgpt|copilot|devin|aider|gemini|windsurf|replit'

usage() {
  cat <<'EOF'
Usage:
  no-ai-attribution-gate.sh --range <git rev-list args>
  no-ai-attribution-gate.sh --message-file <path>
  no-ai-attribution-gate.sh --selftest
  no-ai-attribution-gate.sh --repo <dir> ...

Rejects:
  - a message line starting with "Generated with ..."
  - a Co-Authored-By line naming an AI assistant
  - an author/committer name or email matching an AI assistant

--range accepts any git rev-list arguments, for example "origin/main..HEAD";
use "<sha> --not --remotes" for a brand-new branch.
EOF
}

die_usage() {
  echo "error: $*" >&2
  usage >&2
  exit 2
}

fail() {
  # fail <reason> <where> <detail>
  echo "RESULT=FAIL NO_AI_ATTRIBUTION_GATE reason=$1 where=$2 detail=$3"
}

scan_message() {
  local msg="$1" where="$2"
  if printf '%s\n' "$msg" | grep -Eiq '^[[:space:]]*Generated with[[:space:]]'; then
    fail generated_with "$where" "message carries a 'Generated with' footer"
    return 1
  fi
  if printf '%s\n' "$msg" | grep -Eiq "^[[:space:]]*Co-Authored-By:.*(${DENY})"; then
    fail ai_coauthor "$where" "Co-Authored-By names an AI assistant"
    return 1
  fi
  return 0
}

scan_identity() {
  local name="$1" email="$2" where="$3"
  if printf '%s <%s>' "$name" "$email" | grep -Eiq "$DENY"; then
    fail ai_identity "$where" "author/committer matches an AI assistant"
    return 1
  fi
  return 0
}

scan_range() {
  local spec="$1" commits c msg an ae cn ce
  local -a rev_args
  read -r -a rev_args <<<"$spec"
  if ! commits="$(git -C "$REPO" rev-list "${rev_args[@]}")"; then
    echo "RESULT=FAIL NO_AI_ATTRIBUTION_GATE reason=bad_range range=$spec"
    return 1
  fi
  for c in $commits; do
    SCANNED=$((SCANNED + 1))
    msg="$(git -C "$REPO" log -1 --format=%B "$c")"
    an="$(git -C "$REPO" log -1 --format=%an "$c")"
    ae="$(git -C "$REPO" log -1 --format=%ae "$c")"
    cn="$(git -C "$REPO" log -1 --format=%cn "$c")"
    ce="$(git -C "$REPO" log -1 --format=%ce "$c")"
    scan_message "$msg" "$c" || return 1
    scan_identity "$an" "$ae" "$c" || return 1
    scan_identity "$cn" "$ce" "$c" || return 1
  done
  return 0
}

scan_file() {
  local path="$1" msg
  if [[ ! -f "$path" ]]; then
    echo "RESULT=FAIL NO_AI_ATTRIBUTION_GATE reason=missing_message_file path=$path"
    return 1
  fi
  msg="$(cat "$path")"
  SCANNED=1
  scan_message "$msg" "message-file" || return 1
  return 0
}

selftest() {
  local tmp failures=0 base bad_gen bad_co bad_id
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-attribution.XXXXXX")"
  git -C "$tmp" init -q
  git -C "$tmp" config user.name Test
  git -C "$tmp" config user.email test@example.com
  git -C "$tmp" commit -q --allow-empty -m "fix: clean baseline"
  base="$(git -C "$tmp" rev-parse HEAD)"
  git -C "$tmp" commit -q --allow-empty -m "fix: generated" -m "Generated with Codebuff 🤖"
  bad_gen="$(git -C "$tmp" rev-parse HEAD)"
  git -C "$tmp" commit -q --allow-empty -m "fix: coauthor" -m "Co-Authored-By: Codebuff <noreply@codebuff.com>"
  bad_co="$(git -C "$tmp" rev-parse HEAD)"
  GIT_AUTHOR_NAME=Freebuff GIT_AUTHOR_EMAIL=freebuff@example.com \
    git -C "$tmp" commit -q --allow-empty -m "fix: identity"
  bad_id="$(git -C "$tmp" rev-parse HEAD)"

  printf 'fix: clean message\n\nBody.\n' >"$tmp/good.msg"
  printf 'fix: bad message\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n' >"$tmp/bad.msg"

  expect() {
    local want="$1" label="$2"
    shift 2
    local rc=0
    "$@" >/dev/null 2>&1 || rc=$?
    if [[ "$rc" -ne "$want" ]]; then
      echo "FAIL: selftest ${label}: expected exit ${want}, got ${rc}"
      return 1
    fi
    echo "PASS: selftest ${label}"
    return 0
  }

  REPO="$tmp"
  expect 0 "clean-range" scan_range "$base" || failures=1
  expect 1 "generated-with" scan_range "${base}..${bad_gen}" || failures=1
  expect 1 "ai-coauthor" scan_range "${base}..${bad_co}" || failures=1
  expect 1 "ai-identity" scan_range "${base}..${bad_id}" || failures=1
  expect 0 "clean-message-file" scan_file "$tmp/good.msg" || failures=1
  expect 1 "bad-message-file" scan_file "$tmp/bad.msg" || failures=1
  REPO="$ROOT"

  rm -rf "$tmp"
  if [[ "$failures" -ne 0 ]]; then
    echo "RESULT=FAIL NO_AI_ATTRIBUTION_GATE_SELFTEST"
    return 1
  fi
  echo "RESULT=PASS NO_AI_ATTRIBUTION_GATE_SELFTEST"
  return 0
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo)
      [[ $# -ge 2 ]] || die_usage "--repo needs a value"
      REPO="$2"
      shift 2
      ;;
    --range)
      [[ $# -ge 2 ]] || die_usage "--range needs a value"
      RANGE="$2"
      shift 2
      ;;
    --message-file)
      [[ $# -ge 2 ]] || die_usage "--message-file needs a value"
      MESSAGE_FILE="$2"
      shift 2
      ;;
    --selftest)
      selftest
      exit $?
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die_usage "unknown argument: $1"
      ;;
  esac
done

git -C "$REPO" rev-parse --git-dir >/dev/null 2>&1 || die_usage "not a git repository: $REPO"

if [[ -n "$MESSAGE_FILE" ]]; then
  scan_file "$MESSAGE_FILE" || exit 1
elif [[ -n "$RANGE" ]]; then
  scan_range "$RANGE" || exit 1
else
  die_usage "--range or --message-file is required"
fi

echo "RESULT=PASS NO_AI_ATTRIBUTION_GATE scanned=${SCANNED}"
