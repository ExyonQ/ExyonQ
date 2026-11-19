#!/usr/bin/env bash
# Real-browser WordPress journey against a WordPress image, addressed by digest.
#
# The container runs the published image; a headless Chromium (Playwright) drives
# install -> login -> dashboard -> logout -> cache behaviour through genuine
# navigation. The browser legs never use curl: only the readiness wait and the
# evidence checks do.
#
# Usage: scripts/gates/wordpress-browser-e2e.sh [image@sha256:digest] [outdir]
#   PORT            host port for the container (default 18082)
#   BROWSER_IMAGE   pinned Playwright image (default mcr.microsoft.com/playwright:v1.65.1-noble)
#   PLAYWRIGHT_VERSION  npm package version inside the browser image (default 1.65.1)
#
# The browser image is an external test oracle, not an ExyonQ build; ExyonQ images
# remain Wolfi-only.
set -euo pipefail

IMAGE="${1:-ghcr.io/exyonq/exyonq-wordpress@sha256:61103d6a57df207b774c83b2d09833db1cdde95d79a09e51f8405e6d3d347edd}"
OUT="${2:-.exyonq-local/evidence/wordpress-browser-e2e}"
PORT="${PORT:-18082}"
BROWSER_IMAGE="${BROWSER_IMAGE:-mcr.microsoft.com/playwright:v1.65.1-noble}"
PLAYWRIGHT_VERSION="${PLAYWRIGHT_VERSION:-1.65.1}"
NAME="wp-browser-e2e-$$"
BASE="http://127.0.0.1:${PORT}"
PW="WP-BROWSER-$(openssl rand -hex 12)"
HERE="$(cd "$(dirname "$0")" && pwd)"

mkdir -p "$OUT"
OUT_ABS="$(cd "$OUT" && pwd)"

cleanup() { docker rm -f "$NAME" >/dev/null 2>&1 || true; }
trap cleanup EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

case "$IMAGE" in
  *@sha256:*) : ;;
  *) fail "image must be pinned by digest, got: $IMAGE" ;;
esac

echo "== image: $IMAGE"
docker run -d --name "$NAME" -p "${PORT}:8080" "$IMAGE" >/dev/null
code=""
for _ in $(seq 1 90); do
  code=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/wp-admin/install.php" || true)
  [ "$code" = "200" ] && break
  sleep 1
done
[ "$code" = "200" ] || fail "installer not reachable ($code)"
echo "READY installer=200"

echo "== browser: $BROWSER_IMAGE (playwright ${PLAYWRIGHT_VERSION})"
set +e
docker run --rm --network host \
  -e "WP_BASE=$BASE" \
  -e "WP_ADMIN_PASSWORD=$PW" \
  -e "WP_OUT=/out" \
  -v "${HERE}/wordpress-browser-e2e.cjs:/src/journey.cjs:ro" \
  -v "${OUT_ABS}:/out" \
  "$BROWSER_IMAGE" \
  sh -c "cp /src/journey.cjs /tmp/journey.cjs && cd /tmp && npm install --no-audit --no-fund --silent playwright@${PLAYWRIGHT_VERSION} >/dev/null 2>&1 && node journey.cjs" \
  2>&1 | tee "$OUT_ABS/browser-journey.log"
rc=${PIPESTATUS[0]}
set -e
[ "$rc" -eq 0 ] || fail "browser journey exited $rc"

# The evidence file is written by the browser run; the shell layer only records
# the artifact identity, which the browser cannot know.
{
  echo "# WordPress real-browser journey"
  echo
  echo "- Image: $IMAGE"
  echo "- Browser: $BROWSER_IMAGE (headless Chromium via Playwright ${PLAYWRIGHT_VERSION})"
  echo "- Journey: installer form -> login form -> wp-admin -> cache headers -> logout -> anonymous cache"
  echo
  sed -n 's/^STEP=\(PASS\|FAIL\) /- [x] /p' "$OUT_ABS/browser-journey.log"
  echo
  echo "- Result: $(grep -o 'BROWSER_JOURNEY=[A-Z]*.*' "$OUT_ABS/browser-journey.log" | tail -1)"
} > "$OUT_ABS/wordpress-browser-e2e.md"

echo "WordPress browser journey PASS ($IMAGE)"
