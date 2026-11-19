#!/usr/bin/env bash
# Migration corpus gate (Transversal — Migradores).
#
# The roadmap promises that real-world NGINX configs (WordPress, Laravel, SPA,
# Home Assistant, n8n) migrate to ExyonQ without intervention. This gate keeps
# that promise honest: every corpus file must import (exit 0), produce an IR
# that validates and compiles a runtime plan, lint clean, and report exactly
# the compatibility warnings recorded in `expectations.txt`.
#
# A directive the importer cannot represent must be *listed* in the
# expectations file with its reason, so growth in the warning surface is a
# visible diff instead of a silent regression. `--strict` is fail-closed on any
# warning (EXY-IMPORT-0003), so each file records the strict exit it must give.
#
# Usage:
#   bash scripts/gates/migration-corpus-gate.sh
#   bash scripts/gates/migration-corpus-gate.sh --selftest
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 2
BIN="${EXYONQCTL_BIN:-$ROOT/target/debug/exyonqctl}"
CORPUS="${MIGRATION_CORPUS:-$ROOT/tests/fixtures/migrations}"
EXPECT="$CORPUS/expectations.txt"

report_field() { # report_field <Name> <file>
  sed -n "s/^$1 = //p" "$2" | head -1
}

unsupported_names() { # unsupported_names <report>
  sed -n 's/.*\[UNSUPPORTED\][^`]*`\([A-Za-z_:]*\)`.*/\1/p' "$1" | sort -u | paste -sd, -
}

# corpus_check <conf> <expected_strict_exit> <expected_unsupported_csv>
corpus_check() {
  local conf=$1 want_strict=$2 want_unsup=$3
  local name out rep strict_rc got_unsup compile errors invalid exact
  name="$(basename "$conf")"
  out="$WORK/${name%.conf}.toml"
  rep="$WORK/${name%.conf}.report.txt"

  "$BIN" config migrate-nginx --input "$conf" --output "$out" --report "$rep" \
    --write --check >"$WORK/${name%.conf}.log" 2>&1
  local rc=$?
  if [[ "$rc" -ne 0 ]]; then
    echo "FAIL $name: migrate-nginx exit $rc"
    tail -3 "$WORK/${name%.conf}.log" | sed 's/^/  /'
    return 1
  fi
  if [[ ! -f "$rep" ]]; then
    echo "FAIL $name: no compatibility report written"
    return 1
  fi

  compile="$(report_field RUNTIMEPLAN_COMPILE_RESULT "$rep")"
  errors="$(report_field ERRORS "$rep")"
  invalid="$(report_field INVALID_SOURCE_COUNT "$rep")"
  exact="$(report_field EXACT_COUNT "$rep")"
  got_unsup="$(unsupported_names "$rep")"
  [[ -z "$got_unsup" ]] && got_unsup="-"

  if [[ "$compile" != "PASS" ]]; then
    echo "FAIL $name: RUNTIMEPLAN_COMPILE_RESULT=$compile (expected PASS)"
    return 1
  fi
  if [[ "$errors" != "0" || "$invalid" != "0" ]]; then
    echo "FAIL $name: errors=$errors invalid_source=$invalid (expected 0/0)"
    return 1
  fi
  if [[ "${exact:-0}" -le 0 ]]; then
    echo "FAIL $name: EXACT_COUNT=$exact (nothing mapped)"
    return 1
  fi

  "$BIN" config lint "$out" >"$WORK/${name%.conf}.lint" 2>&1
  if [[ $? -ne 0 ]]; then
    echo "FAIL $name: generated IR fails config lint"
    tail -3 "$WORK/${name%.conf}.lint" | sed 's/^/  /'
    return 1
  fi

  "$BIN" config migrate-nginx --input "$conf" --output "$WORK/strict-$name.toml" \
    --report "$WORK/strict-$name.report.txt" --write --check --strict >/dev/null 2>&1
  strict_rc=$?
  if [[ "$strict_rc" -ne "$want_strict" ]]; then
    echo "FAIL $name: --strict exit $strict_rc (expectations.txt says $want_strict)"
    return 1
  fi

  if [[ "$got_unsup" != "$want_unsup" ]]; then
    echo "FAIL $name: unsupported=$got_unsup (expectations.txt says $want_unsup)"
    echo "  update tests/fixtures/migrations/expectations.txt with the new reason"
    return 1
  fi

  echo "PASS $name exact=$exact unsupported=$got_unsup strict=$strict_rc"
  return 0
}

selftest() {
  local failures=0 tmp
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/migcorpus-selftest.XXXXXX")" || return 1
  WORK="$tmp"
  mkdir -p "$WORK/corpus"

  # A config that reports an unsupported directive the expectations deny.
  cat >"$WORK/corpus/unexpected.conf" <<'EOF'
server {
    listen 80;
    server_name t.example.com;
    root /var/www/t;
    client_max_body_size 1m;
    location / { try_files $uri $uri/ /index.html; }
}
EOF
  if corpus_check "$WORK/corpus/unexpected.conf" 1 "-" >/dev/null 2>&1; then
    echo "FAIL selftest unexpected-unsupported: gate accepted a drifted warning surface"
    failures=$((failures + 1))
  else
    echo "PASS selftest unexpected-unsupported (gate rejected)"
  fi

  # A config the importer cannot even load.
  cat >"$WORK/corpus/broken.conf" <<'EOF'
server {
    listen 80;
    server_name broken.example.com;
    include does-not-exist.conf;
}
EOF
  if corpus_check "$WORK/corpus/broken.conf" 1 "-" >/dev/null 2>&1; then
    echo "FAIL selftest broken-source: gate accepted an unparsable config"
    failures=$((failures + 1))
  else
    echo "PASS selftest broken-source (gate rejected)"
  fi

  # Warnings present but the expectations claim --strict passes.
  cat >"$WORK/corpus/strict-lie.conf" <<'EOF'
server {
    listen 443 ssl;
    server_name strict.example.com;
    root /var/www/strict;
    ssl_certificate /etc/ssl/s.crt;
    ssl_certificate_key /etc/ssl/s.key;
    location / { try_files $uri $uri/ /index.html; }
}
EOF
  if corpus_check "$WORK/corpus/strict-lie.conf" 0 "-" >/dev/null 2>&1; then
    echo "FAIL selftest strict-exit: gate accepted a wrong --strict expectation"
    failures=$((failures + 1))
  else
    echo "PASS selftest strict-exit (gate rejected)"
  fi

  rm -rf "$tmp"
  if [[ "$failures" -ne 0 ]]; then
    echo "RESULT=FAIL MIGRATION_CORPUS_GATE_SELFTEST"
    return 1
  fi
  echo "RESULT=PASS MIGRATION_CORPUS_GATE_SELFTEST"
  return 0
}

if [[ "${1:-}" == "--selftest" ]]; then
  selftest
  exit $?
fi

if [[ ! -x "$BIN" ]]; then
  echo "FAIL missing binary: $BIN (build it with: cargo build -p exyonqctl --bin exyonqctl)"
  echo "RESULT=FAIL MIGRATION_CORPUS_GATE"
  exit 2
fi
if [[ ! -f "$EXPECT" ]]; then
  echo "FAIL missing expectations file: ${EXPECT#$ROOT/}"
  echo "RESULT=FAIL MIGRATION_CORPUS_GATE"
  exit 2
fi

WORK="$ROOT/target/migration-corpus"
rm -rf "$WORK"
mkdir -p "$WORK"

failures=0
checked=0
while read -r name want_strict want_unsup; do
  [[ -z "${name:-}" || "$name" == \#* ]] && continue
  conf="$CORPUS/$name.conf"
  if [[ ! -f "$conf" ]]; then
    echo "FAIL $name: corpus file missing"
    failures=$((failures + 1))
    continue
  fi
  checked=$((checked + 1))
  corpus_check "$conf" "$want_strict" "$want_unsup" || failures=$((failures + 1))
done <"$EXPECT"

# Every corpus file must be covered by the expectations file.
for conf in "$CORPUS"/*.conf; do
  [[ -f "$conf" ]] || continue
  base="$(basename "${conf%.conf}")"
  if ! grep -qE "^${base}[[:space:]]" "$EXPECT"; then
    echo "FAIL ${base}.conf: present in the corpus but absent from expectations.txt"
    failures=$((failures + 1))
  fi
done

if [[ "$failures" -ne 0 ]]; then
  echo "MIGRATION_CORPUS=FAIL files=$checked failures=$failures"
  echo "RESULT=FAIL MIGRATION_CORPUS_GATE"
  exit 1
fi
echo "MIGRATION_CORPUS=PASS files=$checked"
echo "RESULT=PASS MIGRATION_CORPUS_GATE"
exit 0
