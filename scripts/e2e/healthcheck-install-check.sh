#!/usr/bin/env bash
# Run the installed ExyonQ healthcheck against the real local systemd and HTTP
# endpoint. Intended for a Linux host immediately after package installation.
# It does not start or enable the timer and makes no systemd changes.
set -euo pipefail

HEALTHCHECK="/usr/libexec/exyonq/exyonq-healthcheck.sh"
UNIT="exyonq.service"
URL="http://127.0.0.1:8080/health"
PHP_FPM_UNIT=""

usage() {
  cat <<'EOF'
Usage:
  healthcheck-install-check.sh \
    [--healthcheck PATH] [--unit UNIT] [--url URL] [--php-fpm-unit UNIT]
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --healthcheck) HEALTHCHECK="$2"; shift 2 ;;
    --unit) UNIT="$2"; shift 2 ;;
    --url) URL="$2"; shift 2 ;;
    --php-fpm-unit) PHP_FPM_UNIT="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ "$(uname -s)" == "Linux" ]] || {
  echo "ERROR: this check requires a Linux systemd host" >&2
  exit 2
}
command -v systemctl >/dev/null 2>&1 || {
  echo "ERROR: systemctl not found" >&2
  exit 2
}
[[ -x "$HEALTHCHECK" ]] || {
  echo "ERROR: healthcheck is not executable: $HEALTHCHECK" >&2
  exit 2
}

STATUS="$(mktemp)"
trap 'rm -f "$STATUS"' EXIT

echo "HEALTHCHECK=$HEALTHCHECK"
echo "UNIT=$UNIT"
echo "URL=$URL"
systemctl is-active --quiet "$UNIT" || {
  echo "FAIL unit $UNIT is not active before the healthcheck" >&2
  exit 1
}

set +e
EXYONQ_HEALTHCHECK_STATUS_FILE="$STATUS" \
EXYONQ_HEALTHCHECK_UNITS="$UNIT" \
EXYONQ_HEALTHCHECK_URLS="$URL" \
EXYONQ_HEALTHCHECK_PHP_FPM_UNIT="$PHP_FPM_UNIT" \
  "$HEALTHCHECK"
RC=$?
set -e

cat "$STATUS" 2>/dev/null || true
[[ "$RC" -eq 0 ]] || {
  echo "RESULT=FAIL exit=$RC" >&2
  exit "$RC"
}
grep -Fq "OK unit $UNIT" "$STATUS" || {
  echo "RESULT=FAIL missing OK unit $UNIT" >&2
  exit 1
}
grep -Fq "OK http $URL" "$STATUS" || {
  echo "RESULT=FAIL missing OK http $URL" >&2
  exit 1
}
grep -Eq '^(OK unit .*fpm|SKIP php-fpm)' "$STATUS" || {
  echo "RESULT=FAIL no PHP-FPM result recorded" >&2
  exit 1
}

echo "RESULT=PASS"
