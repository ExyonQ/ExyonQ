#!/usr/bin/env bash
# README-RELEASE-VERSION-NOT-STALE
# Fail closed when README.md pins an ExyonQ release version older than the
# workspace version in Cargo.toml. Runtime dependency versions such as PHP are
# not release pins and are out of scope.
#
# Usage:
#   readme-release-version-gate.sh --selftest
#   readme-release-version-gate.sh --tree DIR
set -euo pipefail
LC_ALL=C
export LC_ALL
set +x

SELFTEST_TMP=""

usage() {
  cat <<'EOF'
Usage:
  readme-release-version-gate.sh --selftest
  readme-release-version-gate.sh --tree DIR
EOF
}

version_lt() {
  local left="$1"
  local right="$2"
  if [[ "$left" == "$right" ]]; then
    return 1
  fi
  [[ "$(printf '%s\n%s\n' "$left" "$right" | sort -V | head -n 1)" == "$left" ]]
}

is_release_line() {
  local content="$1"
  [[ "$content" =~ (^|[^[:alnum:]_])[vV][0-9]+\.[0-9]+ ]] && return 0
  [[ "$content" =~ [Ee]xyon[Qq] ]] && return 0
  [[ "$content" == *"releases/"* ]] && return 0
  [[ "$content" == *"What's in"* ]] && return 0
  [[ "$content" == *"VERSION="* ]] && return 0
  [[ "$content" =~ [Rr][Ee][Ll][Ee][Aa][Ss][Ee] ]] && return 0
  return 1
}

run_check() {
  local tree="$1"
  local readme="$tree/README.md"
  local cargo="$tree/Cargo.toml"
  local current current_mm line lineno content version target
  local failures=0

  [[ -f "$readme" ]] || {
    echo "FAIL: missing $readme" >&2
    return 1
  }
  [[ -f "$cargo" ]] || {
    echo "FAIL: missing $cargo" >&2
    return 1
  }

  current="$(awk -F'"' '
    /^\[workspace\.package\][[:space:]]*$/ { in_section = 1; next }
    /^\[/ { in_section = 0 }
    in_section && /^[[:space:]]*version[[:space:]]*=/ { print $2; exit }
  ' "$cargo")"
  if [[ -z "$current" ]]; then
    echo "FAIL: no [workspace.package] version in $cargo" >&2
    return 1
  fi
  current_mm="${current%.*}"

  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    lineno="${line%%:*}"
    content="${line#*:}"
    is_release_line "$content" || continue
    while IFS= read -r version; do
      [[ -z "$version" ]] && continue
      if [[ "$version" =~ ^[0-9]+\.[0-9]+$ ]]; then
        target="$current_mm"
      else
        target="$current"
      fi
      if version_lt "$version" "$target"; then
        echo "FAIL: $readme:$lineno pins release version $version older than $current" >&2
        printf '  %s\n' "$content" >&2
        failures=$((failures + 1))
      fi
    done < <(grep -oE '[0-9]+\.[0-9]+(\.[0-9]+)?' <<<"$content" || true)
  done < <(grep -nE '[0-9]+\.[0-9]+(\.[0-9]+)?' "$readme" || true)

  if (( failures > 0 )); then
    echo "README_RELEASE_VERSION_GATE=FAIL current=$current stale_matches=$failures"
    return 1
  fi
  echo "README_RELEASE_VERSION_GATE=PASS current=$current"
  return 0
}

selftest() {
  local tmp failures=0 dir
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-readme-release-gate.XXXXXX")"
  SELFTEST_TMP="$tmp"
  trap 'rm -rf "$SELFTEST_TMP"' EXIT

  make_case() {
    local name="$1"
    local body="$2"
    mkdir -p "$tmp/$name"
    printf '[workspace.package]\nversion = "0.9.1"\n' >"$tmp/$name/Cargo.toml"
    printf '%b\n' "$body" >"$tmp/$name/README.md"
  }

  make_case pass-current "## What's in the 0.9.x line\nreleases/latest\nPHP 8.5.12"
  make_case pass-php "Requires PHP 8.5.12"
  make_case pass-current-patch "Current release: 0.9.1"
  make_case fail-old-patch "Public product release: v0.8.0"
  make_case fail-image "docker pull ghcr.io/exyonq/exyonq:0.8.0"
  make_case fail-line "## What's in the 0.8.x line"
  make_case fail-previous-patch "Current release: 0.9.0"

  for dir in pass-current pass-php pass-current-patch; do
    if ! run_check "$tmp/$dir" >"$tmp/$dir.out" 2>&1; then
      echo "SELFTEST_UNEXPECTED_FAIL: $dir"
      cat "$tmp/$dir.out" >&2
      failures=$((failures + 1))
    else
      echo "SELFTEST_OK_PASS: $dir"
    fi
  done

  for dir in fail-old-patch fail-image fail-line fail-previous-patch; do
    if run_check "$tmp/$dir" >"$tmp/$dir.out" 2>&1; then
      echo "SELFTEST_UNEXPECTED_PASS: $dir"
      failures=$((failures + 1))
    else
      echo "SELFTEST_OK_FAILS: $dir"
    fi
  done

  if (( failures > 0 )); then
    echo "README_RELEASE_VERSION_GATE_SELFTEST=FAIL failures=$failures"
    return 1
  fi
  echo "README_RELEASE_VERSION_GATE_SELFTEST=PASS"
  return 0
}

MODE=""
TREE=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --selftest)
      MODE=selftest
      shift
      ;;
    --tree)
      [[ $# -ge 2 ]] || { echo "ERROR: --tree requires a directory" >&2; exit 2; }
      MODE=tree
      TREE="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "ERROR: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

[[ -n "$MODE" ]] || {
  echo "ERROR: mode required (--selftest|--tree DIR)" >&2
  exit 2
}

case "$MODE" in
  selftest)
    selftest
    ;;
  tree)
    [[ -d "$TREE" ]] || {
      echo "ERROR: not a directory: $TREE" >&2
      exit 2
    }
    run_check "$TREE"
    ;;
esac
