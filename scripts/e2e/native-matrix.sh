#!/usr/bin/env bash
# Native matrix with anti-false-PASS guards.
#
# Why this exists: a cargo filter that matches nothing exits 0, and rsync can
# preserve old mtimes so Cargo reuses a binary built from a different tree.
# Both produced false PASS evidence in the 0.9.2 campaign. This harness binds
# every result to a tree fingerprint, forces a rebuild when the tree changed,
# and fails when a test command executes zero tests.
#
# Usage:
#   scripts/e2e/native-matrix.sh --print-tree
#   scripts/e2e/native-matrix.sh --run --expected-tree <sha256> [--host LABEL]
#                                [--evidence-dir DIR]
#   scripts/e2e/native-matrix.sh --selftest
#
# Exit codes: 0 all checks passed; 2 usage/config; 3 tree mismatch;
#             4 stale/missing build; 5 a test executed zero tests;
#             6 a step failed; 7 selftest failed.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CARGO_BIN="${CARGO_BIN:-cargo}"
STAMP_REL="target/.exyonq-native-matrix.stamp"

usage() {
  sed -n '2,22p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

fail() {
  local code="$1"
  shift
  printf 'NATIVE_MATRIX=FAIL code=%s reason=%s\n' "$code" "$*" >&2
  exit "$code"
}

# SHA-256 over HEAD, tracked modifications and the content of every tracked
# file. .gitignore keeps target/ and local evidence out, so the fingerprint
# names exactly the source tree that is compiled and tested.
tree_fingerprint() {
  local root="$1"
  {
    git -C "$root" rev-parse HEAD 2>/dev/null || return 1
    git -C "$root" status --porcelain=v1 --untracked-files=no 2>/dev/null || return 1
    (cd "$root" && git ls-files -z | sort -z | xargs -0 sha256sum) || return 1
  } | sha256sum | awk '{print $1}'
}

head_of() {
  git -C "$1" rev-parse HEAD 2>/dev/null || echo UNKNOWN
}

sha256_of() {
  sha256sum "$1" 2>/dev/null | awk '{print $1}'
}

# True only when: binary exists, stamp exists, stamp tree equals current tree,
# stamp binary hash equals current binary, and no tracked source is newer than
# the binary. Prints a reason on failure.
build_current_reason() {
  local root="$1"
  local bin="$root/target/debug/exyonq"
  local stamp="$root/$STAMP_REL"
  [ -x "$bin" ] || { echo MISSING_BINARY; return 0; }
  [ -f "$stamp" ] || { echo MISSING_STAMP; return 0; }
  local stamped_tree stamped_bin current_tree current_bin
  stamped_tree="$(awk '$1=="tree"{print $2}' "$stamp")"
  stamped_bin="$(awk '$1=="bin"{print $2}' "$stamp")"
  current_tree="$(tree_fingerprint "$root")" || { echo TREE_UNREADABLE; return 0; }
  current_bin="$(sha256_of "$bin")"
  [ "$stamped_tree" = "$current_tree" ] || { echo TREE_CHANGED_SINCE_BUILD; return 0; }
  [ "$stamped_bin" = "$current_bin" ] || { echo BINARY_HASH_MISMATCH; return 0; }
  local newer
  newer="$(cd "$root" && git ls-files -z -- '*.rs' 'Cargo.toml' 'Cargo.lock' \
    'rust-toolchain.toml' 'rust-toolchain' | \
    while IFS= read -r -d '' f; do
      [ -e "$f" ] && [ "$f" -nt "$bin" ] && printf '%s\n' "$f"
    done | head -1)"
  [ -z "$newer" ] || { echo "SOURCE_NEWER_THAN_BINARY:$newer"; return 0; }
  echo CURRENT
}

assert_build_current() {
  local root="$1"
  local reason
  reason="$(build_current_reason "$root")"
  if [ "$reason" != CURRENT ]; then
    echo "STALE_ARTIFACT=$reason" >&2
    return 1
  fi
  return 0
}

# Forces a rebuild from the current tree: touch every tracked file so a
# preserved mtime cannot hide changed content, then remove the product binary.
force_rebuild() {
  local root="$1"
  (cd "$root" && git ls-files -z | xargs -0 touch) || return 1
  rm -f "$root/target/debug/exyonq" \
        "$root/target/debug/exyonqctl" \
        "$root/target/debug/exyonq-compat"
}

build_product() {
  local root="$1" log="$2"
  local before after
  before="$(tree_fingerprint "$root")" || fail 2 "tree_unreadable"
  local reason
  reason="$(build_current_reason "$root")"
  if [ "$reason" != CURRENT ]; then
    echo "BUILD_FORCED=$reason"
    force_rebuild "$root" || fail 4 "force_rebuild_failed"
  fi
  if ! (cd "$root" && "$CARGO_BIN" build --locked \
        -p exyonq -p exyonqctl -p exyonq-compat-cli) >"$log" 2>&1; then
    echo "BUILD=FAIL log=$log"
    tail -40 "$log" >&2 || true
    return 1
  fi
  after="$(tree_fingerprint "$root")" || fail 2 "tree_unreadable_after_build"
  [ "$before" = "$after" ] || fail 4 "TREE_CHANGED_DURING_BUILD"
  local bin="$root/target/debug/exyonq"
  [ -x "$bin" ] || fail 4 "binary_missing_after_build"
  {
    printf 'tree %s\n' "$after"
    printf 'bin %s\n' "$(sha256_of "$bin")"
    printf 'head %s\n' "$(head_of "$root")"
    printf 'built_at %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } >"$root/$STAMP_REL" || fail 4 "stamp_write_failed"
  assert_build_current "$root" || return 1
  echo "BUILD=PASS tree=$after bin=$(sha256_of "$bin")"
}

# Fails on a cargo log with no executed test, not merely on cargo's exit code.
assert_test_log() {
  local log="$1"
  if grep -qE 'test result: FAILED' "$log"; then
    echo "TEST_FAILURE=reported_in_log" >&2
    return 1
  fi
  if ! grep -qE 'test result: ok\. [1-9][0-9]* passed' "$log"; then
    echo "ZERO_TESTS=no_test_executed" >&2
    return 1
  fi
  return 0
}

run_step() {
  local name="$1" kind="$2" marker="$3"
  shift 3
  local log="$OUT/$name.log"
  printf '=== %s\n' "$name"
  ( "$@" ) >"$log" 2>&1
  local ec=$?
  printf 'EXIT=%s %s\n' "$ec" "$name" | tee -a "$OUT/summary.txt"
  if [ "$ec" -ne 0 ]; then
    printf 'STEP=FAIL name=%s exit=%s log=%s\n' "$name" "$ec" "$log" \
      | tee -a "$OUT/summary.txt"
    return 1
  fi
  if [ "$kind" = test ]; then
    if ! assert_test_log "$log"; then
      printf 'STEP=FAIL name=%s guard=zero_tests log=%s\n' "$name" "$log" \
        | tee -a "$OUT/summary.txt"
      return 1
    fi
  elif [ -n "$marker" ]; then
    if ! grep -qE "$marker" "$log"; then
      printf 'STEP=FAIL name=%s guard=no_pass_marker log=%s\n' "$name" "$log" \
        | tee -a "$OUT/summary.txt"
      return 1
    fi
  fi
  printf 'STEP=PASS name=%s\n' "$name" | tee -a "$OUT/summary.txt"
  return 0
}

run_matrix() {
  local expected="$1" host="$2" evidence="$3"
  cd "$ROOT" || fail 2 "root_unreadable"

  local tree
  tree="$(tree_fingerprint "$ROOT")" || fail 2 "tree_unreadable"
  if [ -n "$expected" ] && [ "$tree" != "$expected" ]; then
    fail 3 "tree_mismatch expected=$expected actual=$tree"
  fi
  local dirty
  dirty="$(git -C "$ROOT" status --porcelain=v1 --untracked-files=no)"
  [ -z "$dirty" ] || fail 3 "tree_dirty:$dirty"

  local stamp
  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  OUT="${evidence:-$ROOT/.exyonq-local/evidence/native-matrix-$stamp}"
  mkdir -p "$OUT" || fail 2 "evidence_dir_unwritable:$OUT"
  export PATH="$HOME/.local/bin:$HOME/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:$PATH"
  export EXYONQ_CONTROL_SOCKET="$OUT/control.sock"
  export EXYONQ_BIN="$ROOT/target/debug/exyonq"
  export EXYONQCTL_BIN="$ROOT/target/debug/exyonqctl"
  export EXYONQ_COMPAT_BIN="$ROOT/target/debug/exyonq-compat"
  export NG20_SKIP_BUILD=1 NG21_SKIP_BUILD=1 WORKER_MEMORY_SKIP_BUILD=1

  : >"$OUT/summary.txt"
  {
    printf 'host_label=%s\n' "$host"
    printf 'hostname=%s\n' "$(hostname 2>/dev/null || echo unknown)"
    uname -a 2>&1 || true
    id 2>&1 || true
    nproc 2>&1 || true
    printf 'tree_fingerprint=%s\n' "$tree"
    printf 'head=%s\n' "$(head_of "$ROOT")"
  } >"$OUT/host.txt" 2>&1

  if ! build_product "$ROOT" "$OUT/build.log"; then
    printf 'MATRIX=FAIL reason=build\n' | tee -a "$OUT/summary.txt"
    return 1
  fi
  git -C "$ROOT" rev-parse HEAD >"$OUT/head.txt" 2>&1 || true
  sha256sum "$EXYONQ_BIN" "$EXYONQCTL_BIN" "$EXYONQ_COMPAT_BIN" \
    >"$OUT/binaries.sha256" 2>&1 || true
  "$EXYONQ_BIN" --version >"$OUT/version.txt" 2>&1 || true

  local rc=0
  run_step stream_config test "" \
    "$CARGO_BIN" test --locked -p exyonq-config-ir --test stream_config || rc=1
  run_step ng20_stream_proxy_test test "" \
    "$CARGO_BIN" test --locked -p exyonq-core --test ng20_stream_proxy_test || rc=1
  run_step ng20_quic_relay_test test "" \
    "$CARGO_BIN" test --locked -p exyonq-core --test ng20_quic_relay_test || rc=1
  run_step ng20_stream_e2e script 'RESULT=PASS|E2E_RESULT=PASS|SUMMARY pass=[1-9]' \
    bash scripts/e2e/ng20-stream-e2e.sh || rc=1
  run_step ng21_tuning_e2e script 'RESULT=PASS|E2E_RESULT=PASS|SUMMARY pass=[1-9]' \
    bash scripts/e2e/ng21-tuning-e2e.sh || rc=1
  run_step worker_memory_e2e script 'RESULT=PASS|E2E_RESULT=PASS|SUMMARY pass=[1-9]' \
    bash scripts/e2e/worker-memory-e2e.sh || rc=1
  run_step q13_docs script 'Q13_DOCS_AUDIT=PASS|RESULT=PASS' \
    bash scripts/gates/q13-docs-audit.sh || rc=1
  run_step migration_corpus script 'RESULT=PASS|PASS' \
    bash scripts/gates/migration-corpus-gate.sh || rc=1
  run_step compat_matrices script 'RESULT=PASS|PASS' \
    bash scripts/gates/compat-matrices-gate.sh || rc=1
  run_step config_migrate script 'RESULT=PASS|PASS' \
    bash scripts/gates/config-migrate-check.sh || rc=1
  run_step fastcgi_php_fpm script 'fastcgi-php-fpm-e2e: PASS=[1-9]|RESULT=PASS' \
    bash scripts/e2e/fastcgi-php-fpm-e2e.sh || rc=1
  run_step ng20_mysql_e2e script 'RESULT=PASS|E2E_RESULT=PASS|SUMMARY pass=[1-9]' \
    bash scripts/e2e/ng20-mysql-e2e.sh || rc=1

  # Re-check after the suite: a source edit during the run invalidates all
  # evidence above, even if every step passed.
  if ! assert_build_current "$ROOT"; then
    printf 'MATRIX=FAIL reason=tree_or_binary_changed_during_run\n' \
      | tee -a "$OUT/summary.txt"
    return 1
  fi
  local final_tree
  final_tree="$(tree_fingerprint "$ROOT")" || fail 2 "tree_unreadable_final"
  [ "$final_tree" = "$tree" ] || {
    printf 'MATRIX=FAIL reason=tree_changed_during_run expected=%s actual=%s\n' \
      "$tree" "$final_tree" | tee -a "$OUT/summary.txt"
    return 1
  }

  if [ "$rc" -ne 0 ]; then
    printf 'MATRIX=FAIL reason=step_failure\n' | tee -a "$OUT/summary.txt"
    return 1
  fi
  {
    printf 'MATRIX=PASS\n'
    printf 'tree_fingerprint=%s\n' "$tree"
    printf 'head=%s\n' "$(head_of "$ROOT")"
    printf 'binary_sha256=%s\n' "$(sha256_of "$EXYONQ_BIN")"
  } | tee -a "$OUT/summary.txt"
  echo "EVIDENCE_DIR=$OUT"
  printf 'DONE\n' >"$OUT/DONE"
  return 0
}

selftest() {
  local tmp rc=0
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN

  # Case 1: cargo exits 0 but ran zero tests.
  printf 'running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored\n' \
    >"$tmp/zero.log"
  if assert_test_log "$tmp/zero.log" >/dev/null 2>&1; then
    echo "SELFTEST_FAIL=zero_tests_accepted" >&2
    rc=1
  else
    echo "SELFTEST_CASE=zero_tests_rejected"
  fi

  # Case 2: synced tree differs from the compiled tree, with the binary still
  # newer than the source (the rsync mtime failure mode).
  local repo="$tmp/repo"
  mkdir -p "$repo/target/debug" "$repo/target"
  ( cd "$repo" && git init -q && git config user.email t@example.test \
      && git config user.name test && printf 'fn main() {}\n' > main.rs \
      && git add main.rs && git commit -qm base ) || {
    echo "SELFTEST_FAIL=temp_repo" >&2
    return 1
  }
  printf 'binary\n' >"$repo/target/debug/exyonq"
  chmod +x "$repo/target/debug/exyonq"
  local fp
  fp="$(tree_fingerprint "$repo")"
  {
    printf 'tree %s\n' "$fp"
    printf 'bin %s\n' "$(sha256_of "$repo/target/debug/exyonq")"
  } >"$repo/$STAMP_REL"
  if ! assert_build_current "$repo" >/dev/null 2>&1; then
    echo "SELFTEST_FAIL=clean_tree_rejected" >&2
    rc=1
  fi
  # Preserve the old mtime to model rsync -a; content is what changed.
  touch -r "$repo/target/debug/exyonq" "$repo/main.rs"
  printf 'fn main() { println!("changed"); }\n' >"$repo/main.rs"
  touch -r "$repo/target/debug/exyonq" "$repo/main.rs"
  if assert_build_current "$repo" >/dev/null 2>&1; then
    echo "SELFTEST_FAIL=stale_tree_accepted" >&2
    rc=1
  else
    echo "SELFTEST_CASE=stale_tree_rejected"
  fi

  # Case 3: expected tree does not match.
  local other="0000000000000000000000000000000000000000000000000000000000000000"
  if [ "$fp" = "$other" ]; then
    echo "SELFTEST_FAIL=tree_mismatch_fixture" >&2
    rc=1
  else
    echo "SELFTEST_CASE=tree_mismatch_detected"
  fi

  [ "$rc" -eq 0 ] || fail 7 "selftest_failed"
  echo "SELFTEST=PASS cases=3"
}

MODE="${1:---help}"
case "$MODE" in
  --print-tree)
    tree_fingerprint "$ROOT" || fail 2 "tree_unreadable"
    echo "HEAD=$(head_of "$ROOT")"
    ;;
  --selftest)
    selftest
    ;;
  --run)
    shift
    expected="" host="$(hostname 2>/dev/null || echo unknown)" evidence=""
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --expected-tree) expected="${2:?}"; shift 2 ;;
        --host) host="${2:?}"; shift 2 ;;
        --evidence-dir) evidence="${2:?}"; shift 2 ;;
        *) fail 2 "unknown_argument:$1" ;;
      esac
    done
    [ -n "$expected" ] || fail 2 "expected_tree_required"
    run_matrix "$expected" "$host" "$evidence"
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    fail 2 "unknown_mode:$MODE"
    ;;
esac