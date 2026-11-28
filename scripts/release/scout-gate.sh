#!/usr/bin/env bash
# Docker Scout publishing gate.
#
# Owner rule (2026-11-28): an ExyonQ image is not published until Docker Scout
# reports no vulnerabilities for it. When a scan finds fixable ones, the base
# image digests are refreshed, the image is rebuilt and the scan is repeated.
#
# This script is the executable form of that rule and runs anywhere Docker and
# the Scout CLI are installed (the Linux build hosts and CI). It is intentionally
# strict: the default policy is `any`, so ANY reported vulnerability fails the
# gate; `--policy fixable` fails only on vulnerabilities a rebuild can remove and
# is what CI uses, because a brand-new Wolfi base can still carry a CVE that the
# distribution has not patched yet.
#
# Usage:
#   bash scripts/release/scout-gate.sh --image REF [--platform linux/amd64]
#        [--policy any|fixable] [--report FILE]
#
# Examples:
#   bash scripts/release/scout-gate.sh --image ghcr.io/exyonq/exyonq:0.9.3
#   bash scripts/release/scout-gate.sh --image exyonq/exyonq-wordpress:0.9.3 \
#     --platform linux/arm64 --policy fixable
set -uo pipefail
LC_ALL=C
export LC_ALL

IMAGE=""
PLATFORM=""
POLICY="any"
REPORT=""

usage() {
  cat <<'EOF'
Usage: scout-gate.sh --image REF [--platform PLATFORM] [--policy any|fixable]
                     [--report FILE]

  --image REF      Image reference or digest to scan (required).
  --platform P     Scan one platform, e.g. linux/amd64 or linux/arm64.
  --policy any     Fail on any reported vulnerability (default).
  --policy fixable Fail only when Docker Scout reports fixable vulnerabilities.
  --report FILE    Also write the Scout output to FILE.
EOF
}

die_usage() {
  echo "ERROR: $1" >&2
  usage >&2
  exit 2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --image) [[ -n "${2:-}" ]] || die_usage "missing value for --image"; IMAGE="$2"; shift 2 ;;
    --platform) [[ -n "${2:-}" ]] || die_usage "missing value for --platform"; PLATFORM="$2"; shift 2 ;;
    --policy) [[ -n "${2:-}" ]] || die_usage "missing value for --policy"; POLICY="$2"; shift 2 ;;
    --report) [[ -n "${2:-}" ]] || die_usage "missing value for --report"; REPORT="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die_usage "unknown argument: $1" ;;
  esac
done

[[ -n "$IMAGE" ]] || die_usage "--image is required"
case "$POLICY" in
  any|fixable) ;;
  *) die_usage "invalid --policy: $POLICY (expected any or fixable)" ;;
esac

command -v docker >/dev/null 2>&1 || { echo "ERROR: docker not found" >&2; exit 3; }
docker scout version >/dev/null 2>&1 || { echo "ERROR: docker scout not installed" >&2; exit 3; }

args=(cves --exit-code)
if [[ "$POLICY" == "fixable" ]]; then
  args+=(--only-fixed)
fi
if [[ -n "$PLATFORM" ]]; then
  args+=(--platform "$PLATFORM")
fi
args+=("$IMAGE")

tmp="$(mktemp "${TMPDIR:-/tmp}/exyonq-scout.XXXXXX")"
trap 'rm -f "$tmp"' EXIT

echo "DOCKER_SCOUT_SCAN image=${IMAGE} platform=${PLATFORM:-native} policy=${POLICY}"
set +e
docker scout "${args[@]}" >"$tmp" 2>&1
rc=$?
set -e

# Show the summary lines without dumping the whole report.
grep -E 'vulnerabilit|packages|Detected|No vulnerable|Image Reference|digest|platform' "$tmp" | head -20 || true

if [[ -n "$REPORT" ]]; then
  cp "$tmp" "$REPORT"
  echo "DOCKER_SCOUT_REPORT=${REPORT}"
fi

if [[ "$rc" -eq 0 ]]; then
  echo "DOCKER_SCOUT_GATE=PASS image=${IMAGE} policy=${POLICY}"
  exit 0
fi

echo "DOCKER_SCOUT_GATE=FAIL image=${IMAGE} policy=${POLICY}"

# Collect the fixable count, which is the actionable number.
fixable="$(grep -c 'fix:' "$tmp" || true)"
echo "DOCKER_SCOUT_FIXABLE_ENTRIES=${fixable}"
if [[ "$fixable" -gt 0 ]]; then
  echo "NEXT: refresh the pinned Wolfi/Chainguard base digests (packaging/docker/Dockerfile,"
  echo "      packaging/docker/wordpress/Dockerfile), rebuild and scan again."
fi
exit 1
