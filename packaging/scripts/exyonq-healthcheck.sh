#!/bin/sh
# ExyonQ systemd healthcheck.
#
# Checks the configured ExyonQ units and HTTP probes, then discovers the active
# PHP-FPM unit instead of hardcoding a versioned unit name. A non-PHP host with
# no installed php*fpm* unit skips the PHP-FPM check; an installed but inactive
# unit fails the check and lists every candidate that was considered.
#
# Configuration (environment, normally /etc/exyonq/healthcheck.env):
#   EXYONQ_HEALTHCHECK_UNITS="exyonq.service exyonq-http.service"
#   EXYONQ_HEALTHCHECK_UNITS=none                        # no systemd checks at all
#   EXYONQ_HEALTHCHECK_PHP_FPM_UNIT="php-fpm.service"  # optional exact unit
#   EXYONQ_HEALTHCHECK_PHP_FPM_UNIT=off                # disable discovery
#   EXYONQ_HEALTHCHECK_PHP_FPM_PATTERN="php*fpm*"
#   EXYONQ_HEALTHCHECK_PHP_FPM_SOCKET=/run/php/php-fpm.sock  # optional socket
#   EXYONQ_HEALTHCHECK_URLS="https://www.example.com/health"
#   EXYONQ_HEALTHCHECK_TIMEOUT=5
#   EXYONQ_HEALTHCHECK_STATUS_FILE=/var/lib/exyonq/status.txt
#
# HTTPS URLs are verified by curl, including certificate validity and hostname.
# Only a 2xx response is healthy. Redirects are not followed; point the probe at
# the final health endpoint.
#
# Exit: 0 all checks passed; 1 at least one check failed; 2 setup error.
set -u
umask 027

: "${EXYONQ_HEALTHCHECK_STATUS_FILE:=/var/lib/exyonq/status.txt}"
: "${EXYONQ_HEALTHCHECK_UNITS:=exyonq.service}"
: "${EXYONQ_HEALTHCHECK_PHP_FPM_PATTERN:=php*fpm*}"
: "${EXYONQ_HEALTHCHECK_PHP_FPM_UNIT:=}"
: "${EXYONQ_HEALTHCHECK_PHP_FPM_SOCKET:=}"
: "${EXYONQ_HEALTHCHECK_URLS:=http://127.0.0.1:8080/health}"
: "${EXYONQ_HEALTHCHECK_TIMEOUT:=5}"

units="$EXYONQ_HEALTHCHECK_UNITS"
php_unit="$EXYONQ_HEALTHCHECK_PHP_FPM_UNIT"
php_socket="$EXYONQ_HEALTHCHECK_PHP_FPM_SOCKET"
php_pattern="$EXYONQ_HEALTHCHECK_PHP_FPM_PATTERN"
urls="$EXYONQ_HEALTHCHECK_URLS"
timeout="$EXYONQ_HEALTHCHECK_TIMEOUT"
status_file="$EXYONQ_HEALTHCHECK_STATUS_FILE"

report="$(mktemp "${TMPDIR:-/tmp}/exyonq-healthcheck.XXXXXX")" || {
  echo "exyonq-healthcheck: cannot create report" >&2
  exit 2
}
cleanup() {
  rm -f "$report"
}
trap cleanup EXIT HUP INT TERM

failures=0
record() {
  printf '%s\n' "$*" >>"$report"
}
fail() {
  failures=$((failures + 1))
  record "FAIL $*"
}

record "# exyonq-healthcheck $(date -u '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || echo unknown)"

matching_units() {
  # $1: active | known
  if [ "$1" = "active" ]; then
    systemctl list-units --type=service --state=active --no-legend --plain 2>/dev/null
  else
    systemctl list-unit-files --type=service --no-legend --plain 2>/dev/null
  fi | awk '{print $1}' | while IFS= read -r unit; do
    case "$unit" in
      $php_pattern) printf '%s\n' "$unit" ;;
    esac
  done
}

check_php_fpm_units() {
  for unit in $units; do
    if systemctl is-active --quiet "$unit" 2>/dev/null; then
      record "OK unit $unit"
    else
      fail "unit $unit not active"
    fi
  done

  # Keep candidate lists on one line for status.txt and failure messages.
  php_known="$(matching_units known | tr '\n' ' ' | sed 's/ *$//')"
  case "$php_unit" in
    off|none|disabled)
      record "SKIP php-fpm disabled by EXYONQ_HEALTHCHECK_PHP_FPM_UNIT"
      ;;
    "")
      php_active="$(matching_units active | tr '\n' ' ' | sed 's/ *$//')"
      if [ -n "$php_active" ]; then
        for unit in $php_active; do
          if systemctl is-active --quiet "$unit" 2>/dev/null; then
            record "OK unit $unit"
          else
            fail "unit $unit not active"
          fi
        done
      elif [ -n "$php_known" ]; then
        fail "php-fpm no active unit (pattern=$php_pattern); candidates: $php_known"
      else
        record "SKIP php-fpm no installed unit matching $php_pattern"
      fi
      ;;
    *)
      candidates="$php_known"
      case " $candidates " in
        *" $php_unit "*) ;;
        *) candidates="${candidates:+$candidates }$php_unit" ;;
      esac
      if systemctl is-active --quiet "$php_unit" 2>/dev/null; then
        record "OK unit $php_unit"
      else
        fail "unit $php_unit not active; candidates: ${candidates:-$php_unit}"
      fi
      ;;
  esac
}

units_enabled=1
case "$units" in
  ""|none|off|disabled) units_enabled=0 ;;
esac

if [ "$units_enabled" -eq 0 ]; then
  record "SKIP systemd unit checks disabled (EXYONQ_HEALTHCHECK_UNITS=$units)"
elif ! command -v systemctl >/dev/null 2>&1; then
  fail "systemctl not found"
else
  check_php_fpm_units
fi

# Optional PHP-FPM socket probe: the socket must exist and be a socket. The
# request path is still exercised by the HTTP probes below, which is what proves
# PHP-FPM actually answers.
if [ -n "$php_socket" ]; then
  if [ -S "$php_socket" ]; then
    record "OK php-fpm socket $php_socket"
  elif [ -e "$php_socket" ]; then
    fail "php-fpm socket $php_socket is not a socket"
  else
    fail "php-fpm socket missing: $php_socket"
  fi
fi

if ! command -v curl >/dev/null 2>&1; then
  fail "curl not found"
elif [ -z "$urls" ]; then
  fail "EXYONQ_HEALTHCHECK_URLS is empty"
else
  case "$timeout" in
    ''|*[!0-9]*)
      fail "invalid EXYONQ_HEALTHCHECK_TIMEOUT=$timeout"
      ;;
    *)
      for url in $urls; do
        if code=$(curl --silent --show-error --max-time "$timeout" --output /dev/null --write-out '%{http_code}' "$url"); then
          case "$code" in
            2??) record "OK http $url status=$code" ;;
            *) fail "http $url status=$code" ;;
          esac
        else
          rc=$?
          fail "http $url curl_exit=$rc"
        fi
      done
      ;;
  esac
fi

write_status() {
  status_dir="$(dirname "$status_file")"
  if ! mkdir -p "$status_dir" 2>/dev/null; then
    echo "exyonq-healthcheck: cannot create status directory $status_dir" >&2
    return 1
  fi
  status_tmp="${status_file}.tmp.$$"
  if cp "$report" "$status_tmp" 2>/dev/null && mv "$status_tmp" "$status_file" 2>/dev/null; then
    return 0
  fi
  rm -f "$status_tmp" 2>/dev/null || true
  echo "exyonq-healthcheck: cannot write status file $status_file" >&2
  return 1
}

if ! write_status; then
  failures=$((failures + 1))
fi

cat "$report"
if [ "$failures" -ne 0 ]; then
  exit 1
fi
exit 0
