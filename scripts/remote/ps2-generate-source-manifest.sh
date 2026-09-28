#!/usr/bin/env bash
# PS2 — origin-side source bundle manifest (transferred with rsync; remote needs no .git).
set -euo pipefail

WORKSPACE="${1:-.}"
OUT="${2:-$WORKSPACE/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json}"
cd "$WORKSPACE"

STAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RUN_ID="${PS2_RUN_ID:-unknown}"

git_ok=0
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  git_ok=1
fi

if [[ "$git_ok" == "1" ]]; then
  head_sha="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
  branch="$(git branch --show-current 2>/dev/null || echo detached)"
  dirty_lines="$(git status --short 2>/dev/null | wc -l | tr -d ' ')"
  status_hash="$(
    git status --short 2>/dev/null | LC_ALL=C sort | sha256sum | awk '{print $1}'
  )"
else
  head_sha="rsync-no-git"
  branch="rsync-no-git"
  dirty_lines="0"
  status_hash="e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
fi

cargo_lock_sha="$(sha256sum Cargo.lock 2>/dev/null | awk '{print $1}' || echo missing)"
cargo_toml_sha="$(sha256sum Cargo.toml 2>/dev/null | awk '{print $1}' || echo missing)"

bundle_paths=(
  core/src core/tests module-api/src
  crates/exyonq-mod-static crates/exyonq-mod-proxy crates/exyonq-mod-fastcgi
  crates/exyonq-mod-tls crates/exyonq-mod-http3
  crates/exyonq-kernel-contract-consumer crates/exyonq-runtime-plan
  benchmarks/configs benchmarks/docker benchmarks/scenarios
  docs/benchmarks/platform-split/ps2-collect-fingerprint.sh
  docs/benchmarks/platform-split/ps2-run-baseline.sh
  docs/benchmarks/platform-split/ps2-summarize-stats.py
  docs/benchmarks/platform-split/PS2_HOST_POLICY.md
  scripts/remote/ps2-tree-fingerprint.sh
  scripts/remote/validate-ps2-baseline.sh
  scripts/remote/run-ps2-baseline-freeze.sh
  scripts/remote/ps2-generate-source-manifest.sh
  scripts/remote/ps2-verify-remote-identity.sh
)

LIST_FILE="$(mktemp)"
find "${bundle_paths[@]}" -type f 2>/dev/null | LC_ALL=C sort >"$LIST_FILE"

bundle_hash="$(
  xargs -r sha256sum <"$LIST_FILE" 2>/dev/null | sha256sum | awk '{print $1}'
)"
if [[ -z "${bundle_hash:-}" ]]; then
  bundle_hash="$(
    xargs sha256sum <"$LIST_FILE" 2>/dev/null | sha256sum | awk '{print $1}'
  )"
fi
[[ -n "${bundle_hash:-}" ]] || {
  echo "ERROR: bundle_hash empty" >&2
  rm -f "$LIST_FILE"
  exit 2
}

bundle_only_fingerprint="$(printf '%s\n%s' "$bundle_hash" "$cargo_lock_sha" | sha256sum | awk '{print $1}')"

mkdir -p "$(dirname "$OUT")"
FILES_JSON="$(mktemp)"
{
  echo '['
  first=1
  while IFS= read -r f; do
    [[ -f "$f" ]] || continue
    h="$(sha256sum "$f" | awk '{print $1}')"
    if [[ "$first" == "1" ]]; then first=0; else echo ','; fi
    printf '  {"path":"%s","sha256":"%s"}' "$(printf '%s' "$f" | sed 's/"/\\"/g')" "$h"
  done <"$LIST_FILE"
  echo
  echo ']'
} >"$FILES_JSON"
rm -f "$LIST_FILE"

cat >"$OUT" <<EOF
{
  "schema": "ps2-source-bundle-manifest/v1",
  "run_id": "$RUN_ID",
  "generated_at_utc": "$STAMP",
  "origin_hostname": "$(hostname 2>/dev/null || echo unknown)",
  "source_identity": {
    "branch": "$branch",
    "head": "$head_sha",
    "dirty_lines": $dirty_lines,
    "status_hash": "$status_hash",
    "git_captured_on_origin": $([[ "$git_ok" == "1" ]] && echo true || echo false)
  },
  "bundle_hash": "$bundle_hash",
  "bundle_only_fingerprint": "$bundle_only_fingerprint",
  "cargo_lock_sha256": "$cargo_lock_sha",
  "cargo_toml_sha256": "$cargo_toml_sha",
  "files": $(cat "$FILES_JSON")
}
EOF
rm -f "$FILES_JSON"

manifest_hash="$(sha256sum "$OUT" | awk '{print $1}')"
printf '%s\n' "$manifest_hash" >"${OUT}.sha256"
echo "manifest_path=$OUT"
echo "manifest_sha256=$manifest_hash"
echo "bundle_hash=$bundle_hash"
echo "bundle_only_fingerprint=$bundle_only_fingerprint"
