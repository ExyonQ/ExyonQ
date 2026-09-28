#!/usr/bin/env bash
# ZF-005: ExyonQ-owned code must not contain mock infrastructure or mock-upstream naming.
# POLICY: MOCK = FORBIDDEN; SAFE_TEST_DOUBLE = NOT_ALLOWED
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MODE="${1:---tree}"
TREE="${2:-$ROOT}"

fail=0
hits=0

# Encode forbidden tokens so this gate file itself is not a positive hit when scanned.
tok() { printf '%s' "$1"; }

FORBIDDEN_PATTERNS=(
  "$(tok 'mock')-upstream"
  "$(tok 'mock')_upstream"
  "$(tok 'MOCK')_UPSTREAM"
  "exyonq-$(tok 'mock')-upstream"
  "r3-p4-$(tok 'mock')"
  "Dockerfile\\.$(tok 'mock')-upstream"
  "ensure_$(tok 'mock')_upstream"
  "benchmarks/$(tok 'mock')-upstream"
  "bench-http-upstream"
  "exyonq-bench-http-upstream"
  "$(tok 'Mock')FcgiExecutor"
  "$(tok 'Mock')Lifecycle"
  "$(tok 'Mock')Lease"
  "$(tok 'Mock')Server"
  "$(tok 'Mock')Upstream"
  "$(tok 'mock')-server"
  "$(tok 'mock')_server"
  "$(tok 'mock')-purge-server"
  "install_$(tok 'mock')_executor"
)

EXCLUDE_ARGS=(
  --glob '!.git/**'
  --glob '!**/target/**'
  --glob '!.exyonq-local/**'
  --glob '!.cursor/**'
  --glob '!benchmarks/results/**'
  --glob '!benchmarks/bv04/runs/**'
  --glob '!graphify-out/**'
  --glob '!dist/**'
  --glob '!**/historical/**'
  --glob '!**/*.HISTORICAL.md'
  --glob '!scripts/gates/exyonq-owned-mock-references-gate.sh'
  --glob '!scripts/gates/fixtures/**'
  --glob '!scripts/integrity/fixtures/**'
  --glob '!scripts/integrity/lib/**'
  --glob '!scripts/gates/lib/**'
  --glob '!docs/governance/**'
)

if [[ "$MODE" == "--selftest" ]]; then
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/tools/bad"
  # Poison with reconstructed forbidden name
  echo "service: $(tok 'mock')-upstream" >"$tmp/tools/bad/compose.yml"
  if bash "$0" --tree "$tmp"; then
    echo "SELFTEST_FAIL: expected FAIL on poisoned tree"
    exit 1
  fi
  tmp2=$(mktemp -d)
  mkdir -p "$tmp2/tools/upstream"
  echo 'service: upstream' >"$tmp2/tools/upstream/ok.yml"
  if ! bash "$0" --tree "$tmp2"; then
    echo "SELFTEST_FAIL: expected PASS on clean tree"
    exit 1
  fi
  tmp3=$(mktemp -d)
  mkdir -p "$tmp3/bin" "$tmp3/tree"
  echo "service: $(tok 'mock')-upstream" >"$tmp3/tree/bad.yml"
  cat >"$tmp3/bin/rg" <<'STUB'
#!/usr/bin/env bash
exit 127
STUB
  chmod +x "$tmp3/bin/rg"
  if PATH="$tmp3/bin:$PATH" bash "$0" --tree "$tmp3/tree"; then
    echo "SELFTEST_FAIL: expected FAIL when rg broken"
    exit 1
  fi
  echo "MOCK_REFERENCES_GATE_SELFTEST = PASS"
  exit 0
fi

if ! command -v rg >/dev/null 2>&1; then
  echo "SCANNER_REQUIRED=rg"
  echo "MOCK_REFERENCES_GATE = FAIL"
  exit 1
fi

for pat in "${FORBIDDEN_PATTERNS[@]}"; do
  set +e
  out="$(rg -n --no-messages -i "$pat" "$TREE" "${EXCLUDE_ARGS[@]}" 2>/dev/null)"
  rc=$?
  set -e
  if [[ "$rc" -gt 1 ]]; then
    echo "SCANNER_FAILURE: rg exit=$rc pattern=$pat"
    echo "MOCK_REFERENCES_GATE = FAIL"
    exit 1
  fi
  if [[ "$rc" -eq 0 ]]; then
    while IFS= read -r line; do
      [[ -z "$line" ]] && continue
      echo "EXYONQ_OWNED_MOCK_REF: $line"
      hits=$((hits + 1))
      fail=1
    done <<<"$out"
  fi
done

# Also fail on bare word 'mock' in ExyonQ-owned operational zones (narrow globs).
set +e
out="$(rg -n --no-messages -i '\bmock(ed|ing)?\b' "$TREE" \
  --glob 'core/**' \
  --glob 'crates/**' \
  --glob 'modules/**' \
  --glob 'cli/**' \
  --glob 'tools/**' \
  --glob 'integrations/**' \
  --glob 'scripts/e2e/**' \
  --glob 'scripts/remote/**' \
  --glob 'scripts/allocator/**' \
  --glob 'scripts/r3/**' \
  --glob 'scripts/soak/**' \
  --glob 'benchmarks/**' \
  "${EXCLUDE_ARGS[@]}" 2>/dev/null)"
rc=$?
set -e
if [[ "$rc" -gt 1 ]]; then
  echo "SCANNER_FAILURE: rg exit=$rc bare-mock scan"
  echo "MOCK_REFERENCES_GATE = FAIL"
  exit 1
fi
if [[ "$rc" -eq 0 ]]; then
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    echo "EXYONQ_OWNED_MOCK_REF: $line"
    hits=$((hits + 1))
    fail=1
  done <<<"$out"
fi

if [[ "$fail" -ne 0 ]]; then
  echo "EXYONQ_OWNED_MOCK_REFERENCES=$hits"
  echo "MOCK_REFERENCES_GATE = FAIL"
  exit 1
fi

echo "EXYONQ_OWNED_MOCK_REFERENCES=0"
echo "FORBIDDEN_MOCK_UPSTREAM_REFERENCES=0"
echo "MOCK_REFERENCES_GATE = PASS"
