#!/usr/bin/env bash
# COMPAT matrices gate (Transversal — Matrices COMPAT, 0 brechas P1/P2).
#
# Three things must agree, and this gate is where they meet:
#
#   1. what the real importers do  — measured here, every run;
#   2. what tests/fixtures/compat-matrix/expectations.txt says they do;
#   3. what docs/compat/COMPAT_*.md publishes in its `<!-- compat-matrix -->`
#      block.
#
# Every approximated *and* every refused directive must have a line in
# `tests/fixtures/compat-matrix/gaps.txt` with a priority and a status, so
# `P1_OPEN` / `P2_OPEN` are computed from measured gaps instead of asserted:
#
#   * a lossy or refused entry with no manifest line             -> FAIL
#   * a manifest line with priority P1/P2 and status `open`      -> FAIL
#   * a measurement that drifts from expectations.txt            -> FAIL
#   * a published matrix block that drifts from expectations     -> FAIL
#   * a published matrix block that omits a measured metric      -> FAIL
#
# The completeness rule is deliberate: dropping a line from the block (or
# trimming the block to the numbers that flatter it) must be a failure, not a
# silent pass, so `exact=` cannot be removed to hide a regression.
#
# Usage:
#   bash scripts/gates/compat-matrices-gate.sh
#   bash scripts/gates/compat-matrices-gate.sh --selftest
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 2
COMPAT="${EXYONQ_COMPAT_BIN:-$ROOT/target/debug/exyonq-compat}"
CTL="${EXYONQCTL_BIN:-$ROOT/target/debug/exyonqctl}"
MATRIX_DIR="${COMPAT_MATRIX_DIR:-$ROOT/tests/fixtures/compat-matrix}"
EXPECT="${COMPAT_EXPECT:-$MATRIX_DIR/expectations.txt}"
GAPS="${COMPAT_GAPS:-$MATRIX_DIR/gaps.txt}"
DOCS="${COMPAT_DOCS:-$ROOT/docs/compat}"

report_field() { sed -n "s/^$1 = //p" "$2" | head -1; }
report_list_field() { sed -n "s/^- $1: //p" "$2" | head -1; }

matrix_doc() { # matrix_doc <source> [docs-dir]
  local dir="${2:-$DOCS}"
  case "$1" in
    nginx) echo "$dir/COMPAT_NGINX.md" ;;
    apache) echo "$dir/COMPAT_APACHE.md" ;;
    openlitespeed) echo "$dir/COMPAT_OPENLITESPEED.md" ;;
  esac
}

# Machine-readable block of a matrix document: the lines between the markers.
matrix_block() { sed -n '/^<!-- compat-matrix$/,/^-->$/p' "$1" | sed '1d;$d'; }

# ---- measurement: <source>\t<metric>\t<value>, <source>\tgap\t<directive> ----
measure_nginx() {
  local work=$1 corpus="${MIGRATION_CORPUS:-$ROOT/tests/fixtures/migrations}"
  local e=0 l=0 u=0 f rep
  : >"$work/nginx.directives"
  for f in wordpress laravel spa home-assistant n8n; do
    rep="$work/nginx-$f.report.txt"
    "$CTL" config migrate-nginx --input "$corpus/$f.conf" --output "$work/nginx-$f.toml" \
      --report "$rep" --write --check >/dev/null 2>&1
    if [[ $? -ne 0 ]]; then
      echo "FAIL nginx/$f.conf: importer exit non-zero"
      return 1
    fi
    e=$((e + $(report_field EXACT_COUNT "$rep")))
    l=$((l + $(report_field LOSSY_COUNT "$rep")))
    u=$((u + $(report_field UNSUPPORTED_COUNT "$rep")))
    # BSD sed has no alternation: match any status tag, then take the first
    # backticked token of the entry, which is the directive name.
    sed -n 's/^- \[[A-Z_][A-Z_]*\][^`]*`\([A-Za-z_]*\)`.*/\1/p' "$rep" \
      >>"$work/nginx.directives"
    # Both lossy and refused entries need a declared gap: a silent approximation
    # is exactly what "0 brechas" must not hide.
    sed -n 's/.*\[LOSSY\][^`]*`\([A-Za-z_]*\)`.*/nginx\tgap\t\1/p' "$rep"
    sed -n 's/.*\[UNSUPPORTED\][^`]*`\([A-Za-z_]*\)`.*/nginx\tgap\t\1/p' "$rep"
  done
  printf 'nginx\texact\t%s\nnginx\tapproximated\t%s\nnginx\tunsupported\t%s\nnginx\tdirectives\t%s\n' \
    "$e" "$l" "$u" "$(sort -u "$work/nginx.directives" | grep -c .)"
}

measure_apache() {
  local work=$1 rep="$1/apache.report.md"
  "$COMPAT" migrate --from apache --input "$MATRIX_DIR/apache.conf" \
    --out "$work/apache.toml" --report "$rep" >/dev/null 2>&1
  if [[ $? -ne 0 ]]; then
    echo "FAIL apache: importer refused the fixture"
    return 1
  fi
  printf 'apache\texact\t0\napache\tapproximated\t%s\napache\tunsupported\t%s\napache\tdirectives\t%s\n' \
    "$(grep -c '^- \[approximated\]' "$rep")" \
    "$(report_list_field Unsupported "$rep")" \
    "$(grep -vcE '^[[:space:]]*(#|<|$)' "$MATRIX_DIR/apache.conf")"
  sed -n 's/^- \[approximated\] line [0-9]*: \([A-Za-z]*\).*/apache\tgap\t\1/p' "$rep"
  sed -n 's/^- \[unsupported\] line [0-9]*: \([A-Za-z]*\).*/apache\tgap\t\1/p' "$rep"
}

measure_ols() {
  local work=$1 rep="$1/ols.report.md"
  "$COMPAT" ols-throttle --input "$MATRIX_DIR/ols-throttle.conf" \
    --out "$work/ols.toml" --report "$rep" >/dev/null 2>&1
  if [[ $? -ne 0 ]]; then
    echo "FAIL openlitespeed: throttle translation refused"
    return 1
  fi
  if ! grep -q '^\[modules.ratelimit.client\]' "$work/ols.toml"; then
    echo "FAIL openlitespeed: fragment has no [modules.ratelimit.client]"
    return 1
  fi
  printf 'openlitespeed\texact\t0\nopenlitespeed\tapproximated\t%s\nopenlitespeed\tunsupported\t%s\nopenlitespeed\tfields\t%s\n' \
    "$(grep -c '^- \[approximated\]' "$rep")" \
    "$(report_list_field Unsupported "$rep")" \
    "$(grep -cE '^[a-z_]+ = ' "$work/ols.toml")"
  sed -n 's/^- \[approximated\] line [0-9]*: \([A-Za-z]*\).*/openlitespeed\tgap\t\1/p' "$rep"
  sed -n 's/^- \[unsupported\] line [0-9]*: \([A-Za-z]*\).*/openlitespeed\tgap\t\1/p' "$rep"
}

measure() {
  local work="$ROOT/target/compat-matrix"
  rm -rf "$work"; mkdir -p "$work"
  measure_nginx "$work" || return 1
  measure_apache "$work" || return 1
  measure_ols "$work" || return 1
}

# Open-gap counts are per source: `p1_open` in a matrix is that matrix's own
# count, not a global number repeated three times.
open_count() {
  awk -v s="$1" -v p="$2" '!/^#/ && $1==s && $3==p && $4=="open"' "$GAPS" | wc -l | tr -d ' '
}
open_total() {
  awk -v p="$1" '!/^#/ && $3==p && $4=="open"' "$GAPS" | wc -l | tr -d ' '
}

expect_value() { # expect_value <source> <metric>
  awk -v s="$1" -v m="$2" '$1==s && $2==m {print $3}' "$EXPECT" | head -1
}

# ---- verification --------------------------------------------------------
verify() { # verify <facts-file> [docs-dir]
  local facts=$1 docs_dir="${2:-$DOCS}" failures=0 src metric want got p1=0 p2=0 gap_src gap_dir
  local doc key val exp missing

  while read -r src metric want; do
    [[ -z "${src:-}" || "$src" == \#* ]] && continue
    got="$(awk -F'\t' -v s="$src" -v m="$metric" '$1==s && $2==m {print $3}' "$facts" | head -1)"
    if [[ "$got" != "$want" ]]; then
      echo "FAIL matrix drift: $src $metric measured=${got:-none} expected=$want"
      failures=$((failures + 1))
    fi
  done <"$EXPECT"

  while IFS=$'\t' read -r gap_src _ gap_dir; do
    [[ -z "${gap_src:-}" ]] && continue
    if ! grep -qE "^${gap_src}[[:space:]]+${gap_dir}[[:space:]]" "$GAPS"; then
      echo "FAIL undeclared gap: $gap_src $gap_dir (add it to gaps.txt with a reason)"
      failures=$((failures + 1))
    fi
  done < <(grep -E $'\tgap\t' "$facts")

  while read -r src dir prio status rest; do
    [[ -z "${src:-}" || "$src" == \#* ]] && continue
    case "$prio/$status" in
      P1/open)
        failures=$((failures + 1))
        echo "FAIL open P1 gap: $src $dir"
        ;;
      P2/open)
        failures=$((failures + 1))
        echo "FAIL open P2 gap: $src $dir"
        ;;
    esac
  done <"$GAPS"

  # Published matrices must repeat the expectations, not their own history.
  for src in nginx apache openlitespeed; do
    doc="$(matrix_doc "$src" "$docs_dir")"
    if [[ ! -f "$doc" ]]; then
      echo "FAIL missing matrix: ${doc#$ROOT/}"
      failures=$((failures + 1))
      continue
    fi
    # Every published key must exist in the expectations ...
    while IFS='=' read -r key val; do
      case "$key" in
        source) continue ;;
        p1_open|p2_open) continue ;;
        ''|'#'*) continue ;;
      esac
      exp="$(expect_value "$src" "$key")"
      if [[ -z "$exp" ]]; then
        echo "FAIL matrix ${doc##*/}: metric '$key' has no expectation for $src"
        failures=$((failures + 1))
      elif [[ "$val" != "$exp" ]]; then
        echo "FAIL matrix ${doc##*/}: $key=$val but the importer measures $exp"
        failures=$((failures + 1))
      fi
    done < <(matrix_block "$doc")
    # ... and every expectation must be published: a block trimmed to the
    # numbers that flatter it is a regression, not a pass.
    while read -r src_line metric want; do
      [[ "$src_line" == "$src" ]] || continue
      case "$metric" in
        p1_open|p2_open) continue ;;
      esac
      if ! matrix_block "$doc" | grep -qE "^${metric}="; then
        echo "FAIL matrix ${doc##*/}: block omits '$metric' (every expectation must be published)"
        failures=$((failures + 1))
      fi
    done <"$EXPECT"
    local src_p1 src_p2
    src_p1="$(open_count "$src" P1)"
    src_p2="$(open_count "$src" P2)"
    grep -q "^p1_open=$src_p1$" "$doc" || {
      echo "FAIL matrix ${doc##*/}: p1_open does not match the measured $src_p1"
      failures=$((failures + 1))
    }
    grep -q "^p2_open=$src_p2$" "$doc" || {
      echo "FAIL matrix ${doc##*/}: p2_open does not match the measured $src_p2"
      failures=$((failures + 1))
    }
  done

  p1="$(open_total P1)"
  p2="$(open_total P2)"
  echo "COMPAT_MATRICES_GAPS P1_OPEN=$p1 P2_OPEN=$p2"
  if [[ "$failures" -ne 0 ]]; then
    echo "RESULT=FAIL COMPAT_MATRICES_GATE failures=$failures"
    return 1
  fi
  echo "MATRICES=3 P1_OPEN=0 P2_OPEN=0"
  echo "RESULT=PASS COMPAT_MATRICES_GATE"
  return 0
}

selftest() {
  local tmp failures=0 rc
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/compat-matrix-selftest.XXXXXX")" || return 1

  # Baseline from the real corpus, never from a hand-written facts file: the
  # open-P1/P2 lines are downgraded in a *copy* so the baseline exercises the
  # rest of the checks; the unmodified copy is the open-P1 control below.
  if ! measure >"$tmp/facts.real"; then
    echo "FAIL selftest baseline: the measurement did not run"
    rm -rf "$tmp"
    echo "RESULT=FAIL COMPAT_MATRICES_GATE_SELFTEST"
    return 1
  fi
  sed -E 's/^(.*[[:space:]])P[12][[:space:]]+open([[:space:]]|$)/\1P3  workaround\2/' \
    "$GAPS" >"$tmp/gaps.closed"
  # The published blocks must follow the same downgrade, or the baseline would
  # fail on the p1_open comparison instead of on the gate logic under test.
  cp -R "$DOCS" "$tmp/docs-closed"
  sed -i '' 's/^p1_open=1$/p1_open=0/' "$tmp/docs-closed/COMPAT_NGINX.md" 2>/dev/null ||
    sed -i 's/^p1_open=1$/p1_open=0/' "$tmp/docs-closed/COMPAT_NGINX.md"

  if GAPS="$tmp/gaps.closed" verify "$tmp/facts.real" "$tmp/docs-closed" >/dev/null 2>&1; then
    echo "PASS selftest baseline (real measurement, expectations and matrices agree)"
  else
    echo "FAIL selftest baseline: the real corpus is rejected"
    GAPS="$tmp/gaps.closed" verify "$tmp/facts.real" "$tmp/docs-closed" 2>&1 | sed 's/^/    /'
    failures=$((failures + 1))
  fi

  # Control A: a declared-but-open P1 must fail.
  cp "$tmp/gaps.closed" "$tmp/gaps.p1"
  printf 'nginx keepalive P1 open injected for selftest\n' >>"$tmp/gaps.p1"
  if GAPS="$tmp/gaps.p1" verify "$tmp/facts.real" "$tmp/docs-closed" >/dev/null 2>&1; then
    echo "FAIL selftest open-P1: gate accepted an open P1 gap"
    failures=$((failures + 1))
  else
    echo "PASS selftest open-P1 (gate rejected)"
  fi

  # Control B: an undeclared lossy entry must fail.
  cp "$tmp/facts.real" "$tmp/facts.undeclared"
  printf 'nginx\tgap\tnot_in_manifest\n' >>"$tmp/facts.undeclared"
  if GAPS="$tmp/gaps.closed" verify "$tmp/facts.undeclared" "$tmp/docs-closed" >/dev/null 2>&1; then
    echo "FAIL selftest undeclared-gap: gate accepted a silent gap"
    failures=$((failures + 1))
  else
    echo "PASS selftest undeclared-gap (gate rejected)"
  fi

  # Control C: a drifted expectation must fail.
  cp "$EXPECT" "$tmp/expect.drift"
  awk 'BEGIN{done=0} $1=="nginx" && $2=="exact" && !done {print "nginx exact 0"; done=1; next} {print}' \
    "$EXPECT" >"$tmp/expect.drift"
  if GAPS="$tmp/gaps.closed" EXPECT="$tmp/expect.drift" verify "$tmp/facts.real" "$tmp/docs-closed" >/dev/null 2>&1; then
    echo "FAIL selftest count-drift: gate accepted a drifted expectation"
    failures=$((failures + 1))
  else
    echo "PASS selftest count-drift (gate rejected)"
  fi

  # Control D: a matrix that claims other numbers than the importer measures.
  cp -R "$DOCS" "$tmp/docs"
  # The published value must stay in sync with expectations.txt, so the control
  # rewrites whatever number is there instead of a hard-coded one.
  sed -i '' -E 's/^exact=[0-9]+$/exact=999/' "$tmp/docs/COMPAT_NGINX.md" 2>/dev/null ||
    sed -i -E 's/^exact=[0-9]+$/exact=999/' "$tmp/docs/COMPAT_NGINX.md"
  if GAPS="$tmp/gaps.closed" verify "$tmp/facts.real" "$tmp/docs" >/dev/null 2>&1; then
    echo "FAIL selftest matrix-drift: gate accepted a matrix that contradicts the importer"
    failures=$((failures + 1))
  else
    echo "PASS selftest matrix-drift (gate rejected)"
  fi

  # Control E: a block that drops a metric line must fail (completeness).
  rm -rf "$tmp/docs-trimmed"; cp -R "$tmp/docs-closed" "$tmp/docs-trimmed"
  sed -i '' -E '/^exact=[0-9]+$/d' "$tmp/docs-trimmed/COMPAT_NGINX.md" 2>/dev/null ||
    sed -i -E '/^exact=[0-9]+$/d' "$tmp/docs-trimmed/COMPAT_NGINX.md"
  if GAPS="$tmp/gaps.closed" verify "$tmp/facts.real" "$tmp/docs-trimmed" >/dev/null 2>&1; then
    echo "FAIL selftest block-completeness: gate accepted a block without exact="
    failures=$((failures + 1))
  else
    echo "PASS selftest block-completeness (gate rejected)"
  fi

  # Control F: `return <status> "<body>"` must not lose its mapping silently.
  # The facts get a synthetic lossy `return` entry (what the importer would
  # report if route.respond stopped being emitted) and the manifest declares it
  # P1/open, so the gate has to reopen P1 instead of trusting the baseline.
  cp "$tmp/facts.real" "$tmp/facts.return"
  printf 'nginx\tgap\treturn\n' >>"$tmp/facts.return"
  cp "$tmp/gaps.closed" "$tmp/gaps.return"
  printf 'nginx return P1 open injected for selftest (return body lost its mapping)\n' \
    >>"$tmp/gaps.return"
  out="$(GAPS="$tmp/gaps.return" verify "$tmp/facts.return" "$tmp/docs-closed" 2>&1)"
  if echo "$out" | grep -q "FAIL open P1 gap: nginx return"; then
    echo "PASS selftest return-p1 (gate rejected an unmapped return body)"
  else
    echo "FAIL selftest return-p1: gate accepted an unmapped return body"
    echo "$out" | sed 's/^/    /'
    failures=$((failures + 1))
  fi

  rm -rf "$tmp"
  if [[ "$failures" -ne 0 ]]; then
    echo "RESULT=FAIL COMPAT_MATRICES_GATE_SELFTEST"
    return 1
  fi
  echo "RESULT=PASS COMPAT_MATRICES_GATE_SELFTEST"
  return 0
}

if [[ "${1:-}" == "--selftest" ]]; then
  selftest
  exit $?
fi

for bin in "$CTL" "$COMPAT"; do
  if [[ ! -x "$bin" ]]; then
    echo "FAIL missing binary: ${bin#$ROOT/} (build the workspace first)"
    echo "RESULT=FAIL COMPAT_MATRICES_GATE"
    exit 2
  fi
done

facts="$(mktemp)"
measure >"$facts" || { rm -f "$facts"; echo "RESULT=FAIL COMPAT_MATRICES_GATE"; exit 1; }
verify "$facts" "$DOCS"
rc=$?
rm -f "$facts"
exit $rc
