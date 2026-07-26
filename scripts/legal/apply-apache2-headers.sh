#!/usr/bin/env bash
# Insert Apache-2.0 copyright headers into main workspace Rust sources.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DRY_RUN=0
CHECK=0

usage() {
  cat <<'EOF'
usage: apply-apache2-headers.sh [--dry-run] [--check]

  --dry-run   Print files that would be modified; do not write.
  --check     Exit 1 if any target file lacks an Apache-2.0 header.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --check) CHECK=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage; exit 2 ;;
  esac
  shift
done

read -r -d '' HEADER <<'EOF' || true
/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

EOF

MARKER='Licensed under the Apache License, Version 2.0'

should_skip_path() {
  local path="$1"
  case "$path" in
    tests/*|fuzz/*|benchmarks/*|target/*|*/examples/*|*/benches/*) return 0 ;;
  esac
  return 1
}

in_scope_crate() {
  local path="$1"
  case "$path" in
    core/*|modules/*|cli/*|compat/*|config/*|\
    module-api/*|addon-api/*|addon-sdk/*|wasm/exyonq-wasm-host/*|xtask/*) return 0 ;;
  esac
  return 1
}

collect_files() {
  find "$ROOT" -name '*.rs' -type f ! -path '*/target/*' | while read -r f; do
    rel="${f#"$ROOT"/}"
    should_skip_path "$rel" && continue
    in_scope_crate "$rel" || continue
    echo "$f"
  done
}

has_header() {
  grep -qF "$MARKER" "$1"
}

insert_header() {
  local file="$1"
  local tmp
  tmp="$(mktemp)"

  if head -n1 "$file" | grep -q '^#!'; then
    { head -n1 "$file"; printf '%s' "$HEADER"; tail -n +2 "$file"; } >"$tmp"
  elif head -n1 "$file" | grep -q '^#!\['; then
    { printf '%s' "$HEADER"; cat "$file"; } >"$tmp"
  else
    printf '%s' "$HEADER" >"$tmp"
    cat "$file" >>"$tmp"
  fi

  if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "would update: ${file#"$ROOT"/}"
    rm -f "$tmp"
    return 0
  fi

  mv "$tmp" "$file"
  echo "updated: ${file#"$ROOT"/}"
}

added=0
skipped=0
missing=0

while IFS= read -r file; do
  [[ -z "$file" ]] && continue
  if has_header "$file"; then
    skipped=$((skipped + 1))
    continue
  fi
  if [[ "$CHECK" -eq 1 ]]; then
    echo "missing header: ${file#"$ROOT"/}" >&2
    missing=$((missing + 1))
    continue
  fi
  insert_header "$file"
  added=$((added + 1))
done < <(collect_files)

echo "summary: added=$added skipped=$skipped missing=$missing"
if [[ "$CHECK" -eq 1 && "$missing" -gt 0 ]]; then
  exit 1
fi
