#!/usr/bin/env bash
# Independent verifier for the CLAIM-EVIDENCE-INTEGRITY adversarial cases.
#
# Treats the checker as a black box: it runs the CLI and reads stdout only. It
# reimplements none of the engine's logic, so an engine defect cannot silently
# satisfy this verifier — but it does invoke the same entrypoint, so it proves
# behavior, not independence of implementation.
#
# Three layers:
#
#   1. OWNER CASES. For every row of expectations.tsv: the verdict matches the
#      owner-specified expectation, the finding code the case is about is
#      present, and the counterfactual holds — applying only the declared
#      repair makes the block PASS. Without the counterfactual, a checker that
#      rejects everything would score 100%.
#
#   2. PROBES. One derivation from a known-good template per gate, so every
#      finding code the engine can emit is exercised by something. A gate with
#      no probe is a gate that can be deleted without anyone noticing.
#
#   3. MUTANTS. The engine is copied and one gate at a time is switched off;
#      each mutant must be caught. Two whole-verdict mutants (always FAIL,
#      always PASS) are run against the full verifier; the per-gate mutants are
#      run against the probe layer, which is where their behavior shows.
#
# Exit 0 = all cases behave as specified. Exit 1 = at least one deviation.
set -euo pipefail
LC_ALL=C
export LC_ALL

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INTEGRITY_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
FIXTURES="$INTEGRITY_DIR/fixtures/claim-evidence"
EXPECTATIONS="$FIXTURES/expectations.tsv"
CHECKER="$INTEGRITY_DIR/claim-evidence-check.sh"
ENGINE="$INTEGRITY_DIR/lib/claim_evidence_check.py"
# Mutant copies live outside the repository, so the real root is passed down
# rather than re-derived from a script location that has moved.
REPO_ROOT="${CEI_REPO_ROOT:-$(cd "$INTEGRITY_DIR/../.." && pwd)}"

[[ -f "$EXPECTATIONS" ]] || { echo "ERROR: missing $EXPECTATIONS" >&2; exit 2; }
[[ -f "$CHECKER" ]] || { echo "ERROR: missing $CHECKER" >&2; exit 2; }

TMPDIR_RUN="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_RUN"' EXIT

PROBES_ONLY="${CEI_PROBES_ONLY:-0}"

cases=0
ok=0
bad=0
declare -a FAILURES=()
declare -a ROW_FIXTURES=()
# Every finding code this run actually asserts on. Compared at the end against
# the codes the engine can emit, so a gate cannot exist unexercised.
declare -a ASSERTED_CODES=()

fail_case() {
  bad=$((bad + 1))
  FAILURES+=("$1")
  echo "CASE_RESULT=DEVIATION $1"
}

note_code() { [[ "$1" == "-" || "$1" == !* ]] || ASSERTED_CODES+=("$1"); }

run_checker() { # file -> stdout of checker, exit code ignored here
  set +e
  /bin/bash "$CHECKER" --check "$1"
  set -e
}

field_of() { # output field_name  (first block only; fixtures hold one block)
  printf '%s\n' "$1" | awk -F= -v k="$2" '$1==k {print $2; exit}'
}

field_of_claim() { # output claim_id field_name  (from a multi-block run)
  printf '%s\n' "$1" | awk -F= -v id="$2" -v k="$3" '
    $1=="CLAIM_ID" { inblock = ($2 == id) }
    inblock && $1==k { print $2; exit }
  '
}

apply_one() { # file key value
  awk -v key="$2" -v val="$3" '
    BEGIN { done = 0 }
    {
      probe = $0
      sub(/^[ \t]+/, "", probe)
      if (!done && index(probe, key " =") == 1) {
        print key " = " val
        done = 1
        next
      }
      if (!done && $0 ~ /END_CLAIM_EVIDENCE_BLOCK/) {
        print key " = " val
        done = 1
      }
      print $0
    }
  ' "$1"
}

apply_mutation() { # source_file "K=V;K=V" out_file
  local src="$1" spec="$2" out="$3"
  local work="$TMPDIR_RUN/work.cei"
  cp "$src" "$work"
  local IFS=';'
  local pair
  for pair in $spec; do
    [[ -n "$pair" ]] || continue
    local key="${pair%%=*}"
    local val="${pair#*=}"
    apply_one "$work" "$key" "$val" > "$work.next"
    mv "$work.next" "$work"
  done
  mv "$work" "$out"
}

# --- layer 1: owner cases ----------------------------------------------------

# Owner outcome word -> verdicts that word permits. Keeps the expectation rows
# tied to the wording of the authorization instead of to checker behavior.
verdicts_for_owner_word() {
  case "$1" in
    REJECT|INSUFFICIENT|WRONG_CAUSAL_PATH|STALE_OR_REJECT) echo "FAIL" ;;
    REVIEW_OR_INSUFFICIENT|SELF_CONFIRMING_RISK) echo "FAIL REVIEW" ;;
    NEGATIVE_CONTROL) echo "PASS" ;;
    FAIL_CLOSED) echo "REVIEW" ;;
    *) echo "" ;;
  esac
}

owner_cases() {
  while IFS=$'\t' read -r case_id fixture expected required mutation owner_word spec_ref; do
    [[ -n "${case_id:-}" ]] || continue
    [[ "$case_id" == \#* ]] && continue
    [[ "$case_id" == "CASE_ID" ]] && continue
    cases=$((cases + 1))
    ROW_FIXTURES+=("$fixture")
    note_code "$required"

    path="$FIXTURES/$fixture"
    if [[ ! -f "$path" ]]; then
      fail_case "$case_id: fixture not found: $fixture"
      continue
    fi

    allowed="$(verdicts_for_owner_word "${owner_word:-}")"
    if [[ -z "$allowed" ]]; then
      fail_case "$case_id: unknown OWNER_WORD '${owner_word:-<none>}'"
      continue
    fi
    if [[ " $allowed " != *" $expected "* ]]; then
      fail_case "$case_id: OWNER_WORD $owner_word permits [$allowed] but the row expects $expected"
      continue
    fi

    out="$(run_checker "$path")"
    verdict="$(field_of "$out" CLAIM_EVIDENCE_COMPATIBILITY)"
    findings="$(field_of "$out" FINDINGS)"
    blocks="$(field_of "$out" BLOCKS_TOTAL)"

    case_ok=1
    # field_of reads the first block, so a fixture must hold exactly one.
    if [[ "$blocks" != "1" ]]; then
      fail_case "$case_id: fixture holds ${blocks:-0} blocks, expected exactly 1"
      case_ok=0
    fi
    if [[ "$verdict" != "$expected" ]]; then
      fail_case "$case_id ($spec_ref): expected verdict $expected, checker said ${verdict:-<none>}"
      case_ok=0
    fi

    if [[ "$required" != "-" ]]; then
      if [[ ",$findings," != *",$required,"* ]]; then
        fail_case "$case_id ($spec_ref): expected finding $required, got ${findings:-<none>}"
        case_ok=0
      fi
    fi

    if [[ "$mutation" != "-" ]]; then
      mutated="$TMPDIR_RUN/$case_id.mutated.cei"
      apply_mutation "$path" "$mutation" "$mutated"
      set +e
      mout="$(/bin/bash "$CHECKER" --check "$mutated")"
      mrc=$?
      set -e
      mverdict="$(field_of "$mout" CLAIM_EVIDENCE_COMPATIBILITY)"
      if [[ "$mverdict" != "PASS" ]]; then
        mfindings="$(field_of "$mout" FINDINGS)"
        fail_case "$case_id ($spec_ref): counterfactual failed — after repair [$mutation] verdict is ${mverdict:-<none>} (${mfindings:-<none>}), expected PASS"
        case_ok=0
      elif [[ $mrc -ne 0 ]]; then
        # The exit code is what a CI gate reads; PASS must mean 0.
        fail_case "$case_id ($spec_ref): repaired block reported PASS but exited $mrc, expected 0"
        case_ok=0
      fi
    elif [[ "$expected" != "PASS" ]]; then
      fail_case "$case_id ($spec_ref): non-PASS row must declare a counterfactual mutation"
      case_ok=0
    fi

    if [[ $case_ok -eq 1 ]]; then
      ok=$((ok + 1))
      echo "CASE_RESULT=OK $case_id $expected ${required}"
    fi
  done < "$EXPECTATIONS"

  # Bidirectional coverage: every fixture claimed by exactly one row.
  for f in "$FIXTURES"/*.cei; do
    [[ -e "$f" ]] || continue
    base="$(basename "$f")"
    hits=0
    for rf in "${ROW_FIXTURES[@]}"; do
      [[ "$rf" == "$base" ]] && hits=$((hits + 1))
    done
    if [[ $hits -ne 1 ]]; then
      fail_case "coverage: fixture $base is claimed by $hits expectation rows, expected exactly 1"
    fi
  done
}

# --- layer 2: probes ---------------------------------------------------------

write_base_template() {
  cat > "$1" <<'TPL'
BEGIN_CLAIM_EVIDENCE_BLOCK
CLAIM_ID = PROBE-BASE
CLAIM = The routing table answers an unknown host with 404
CLAIM_KIND = EXECUTION_REPORT
CLAIM_STATUS = PASS
CLAIM_SCOPE = PACKAGE
CLAIM_PLATFORM = DARWIN_ARM64
CLAIM_CLIENT = NOT_APPLICABLE
CLAIM_EXECUTION_CLASS = INTEGRATION
CLAIM_SOURCE_STATE = HEAD=0000dead0000beef STATE=CLEAN
SOURCE_BINDING = ILLUSTRATIVE
CLAIM_REQUIRED_EFFECT = POSITIVE_OBSERVATION the response status is 404
CLAIM_CONCURRENCY_CLASS = NONE
EPISTEMIC_STATUS = PROVEN
CONTRACT_SOURCE = PROJECT_DOC
EVIDENCE = the routing suite for that package
EVIDENCE_COMMAND = cargo test -p exyonq-core routing
EVIDENCE_SCOPE = PACKAGE
EVIDENCE_PLATFORM = DARWIN_ARM64
EVIDENCE_CLIENT = NOT_APPLICABLE
EVIDENCE_EXECUTION_CLASS = INTEGRATION
EVIDENCE_SOURCE_STATE = HEAD=0000dead0000beef STATE=CLEAN
EVIDENCE_EXECUTION = EXECUTED
EVIDENCE_RESULT = PASS
EVIDENCE_ORACLE_ORIGIN = PROJECT_DOC
EVIDENCE_INDEPENDENCE = INDEPENDENT
EVIDENCE_OBSERVED_EFFECT = POSITIVE_OBSERVATION the response status was 404
MIRRORED_IMPLEMENTATION_ORACLE = NO
SELF_CONFIRMING_TEST_LOOP_RISK = LOW
TARGET_CAUSAL_PATH = NOT_APPLICABLE
PATH_EXECUTED = NOT_APPLICABLE
PATH_EXECUTION_EVIDENCE = NOT_APPLICABLE
CONCURRENCY_EVIDENCE_FORM = NONE
CLAIM_EVIDENCE_COMPATIBILITY = PASS
END_CLAIM_EVIDENCE_BLOCK
TPL
}

# NAME | EXPECTED_VERDICT | CODES (comma list, !CODE = must not appear) | OVERRIDES
probe_rows() {
  cat <<'ROWS'
BASE|PASS|-|-
DISCLAIMED-PLATFORM|REVIEW|DIMENSION_DISCLAIMED|CLAIM_PLATFORM=NOT_APPLICABLE
DISCLAIMED-PLATFORM-STATIC|REVIEW|DIMENSION_DISCLAIMED|CLAIM_EXECUTION_CLASS=STATIC_ANALYSIS;EVIDENCE_EXECUTION_CLASS=STATIC_ANALYSIS;CLAIM_PLATFORM=NOT_APPLICABLE
DISCLAIMED-CLIENT-FIELD|REVIEW|DIMENSION_DISCLAIMED|EVIDENCE_CLIENT=CURSOR
DISCLAIMED-PLATFORM-PROSE|REVIEW|DIMENSION_DISCLAIMED|CLAIM_PLATFORM=NOT_APPLICABLE;EVIDENCE_PLATFORM=NOT_APPLICABLE;CLAIM=The proxy answers correctly on Linux amd64 hosts
DISCLAIMED-CLIENT-PROSE|REVIEW|DIMENSION_DISCLAIMED|CLAIM=The tool answers when it is driven from Cursor
DISCLAIMED-SOURCE|REVIEW|SOURCE_BINDING_MISSING|CLAIM_SOURCE_STATE=NOT_APPLICABLE
DISCLAIMED-EFFECT|REVIEW|DIMENSION_DISCLAIMED|CLAIM_REQUIRED_EFFECT=NOT_APPLICABLE
DISCLAIMED-RUNTIME-PLATFORM|REVIEW|DIMENSION_DISCLAIMED|CLAIM_EXECUTION_CLASS=REAL_RUNTIME;EVIDENCE_EXECUTION_CLASS=REAL_RUNTIME;CLAIM_PLATFORM=NOT_APPLICABLE;EVIDENCE_PLATFORM=NOT_APPLICABLE
CAUSAL-DISCLAIMED-SAME-OUTPUT|FAIL|WRONG_CAUSAL_PATH|PATH_EXECUTION_EVIDENCE=SAME_OUTPUT the other branch returns 502 as well
CAUSAL-PROSE|REVIEW|CAUSAL_PATH_REQUIRED|CLAIM=The latency spike is caused by the descriptor table lookup
ROOT-CAUSE-NO-PATH|FAIL|CAUSAL_PATH_REQUIRED|CLAIM_KIND=ROOT_CAUSE
CAUSAL-UNVERIFIED|REVIEW|CAUSAL_PATH_UNVERIFIED|TARGET_CAUSAL_PATH=the truncated upstream body branch;PATH_EXECUTED=UNKNOWN
ORACLE-DERIVED|REVIEW|SELF_CONFIRMING_RISK|EVIDENCE_INDEPENDENCE=IMPLEMENTATION_DERIVED
ORACLE-UNKNOWN|REVIEW|ORACLE_INDEPENDENCE_UNKNOWN|EVIDENCE_INDEPENDENCE=UNKNOWN
SELF-CONFIRMING-MEDIUM|REVIEW|SELF_CONFIRMING_RISK|SELF_CONFIRMING_TEST_LOOP_RISK=MEDIUM
SELF-CONFIRMING-MIRRORED|FAIL|SELF_CONFIRMING_ORACLE|MIRRORED_IMPLEMENTATION_ORACLE=YES
CONTRACT-PROOF-FROM-IMPL|REVIEW|TEST_PASS_AS_CONTRACT_PROOF|CLAIM_KIND=CONTRACT_PROOF;CONTRACT_SOURCE=IMPLEMENTATION
CONCURRENCY-PROSE|REVIEW|CONCURRENCY_CLASS_DIVERGENCE|CLAIM=The snapshot swap is safe under parallel load since check-then-act cannot observe a torn generation
CONCURRENCY-GAP|FAIL|CONCURRENCY_EVIDENCE_GAP|CLAIM_CONCURRENCY_CLASS=TOCTOU;CONCURRENCY_EVIDENCE_FORM=SEQUENTIAL_TEST
UNKNOWN-STATUS-IS-MATERIAL|FAIL|PARTIAL_AS_GLOBAL,UNKNOWN_TOKEN|CLAIM_STATUS=SHIPPED;EPISTEMIC_STATUS=PARTIAL
EPISTEMIC-NOT-PROVEN|FAIL|NOT_PROVEN_AS_PASS|EPISTEMIC_STATUS=NOT_PROVEN
EPISTEMIC-DISPROVEN|FAIL|DISPROVEN_AS_PASS|EPISTEMIC_STATUS=DISPROVEN
EPISTEMIC-BOUNDED|REVIEW|EPISTEMIC_UNDERSUPPORT,DECLARED_VERDICT_INFLATED|EPISTEMIC_STATUS=BOUNDED
RESULT-FAIL|FAIL|RESULT_NOT_PASS|EVIDENCE_RESULT=FAIL
RESULT-UNKNOWN|REVIEW|RESULT_UNKNOWN|EVIDENCE_RESULT=UNKNOWN
EXECUTION-UNKNOWN|REVIEW|EXECUTION_UNKNOWN|EVIDENCE_EXECUTION=UNKNOWN
EXECUTION-PARTIAL|FAIL|PARTIAL_AS_GLOBAL|EVIDENCE_EXECUTION=PARTIALLY_EXECUTED
EXECUTION-NOT-RUN|FAIL|UNEXECUTED_CLAIM|EVIDENCE_EXECUTION=NOT_EXECUTED
DECLARED-FAIL-HONORED|FAIL|DECLARED_VERDICT_HONORED|CLAIM_EVIDENCE_COMPATIBILITY=FAIL
SOURCE-DIRTY|FAIL|SOURCE_STATE_MISMATCH|EVIDENCE_SOURCE_STATE=HEAD=0000dead0000beef STATE=DIRTY
SOURCE-STALE|FAIL|STALE_EVIDENCE|EVIDENCE_SOURCE_STATE=HEAD=1111beef1111dead STATE=CLEAN
SOURCE-SHORT|REVIEW|SHORT_SOURCE_BINDING|CLAIM_SOURCE_STATE=HEAD=abc STATE=CLEAN;EVIDENCE_SOURCE_STATE=HEAD=abc STATE=CLEAN
SOURCE-BRANCH-NAME|REVIEW|UNKNOWN_TOKEN|CLAIM_SOURCE_STATE=HEAD=main STATE=CLEAN
SOURCE-CURRENT-UNBOUND|REVIEW|UNBOUND_SOURCE_STATE|CLAIM_SOURCE_STATE=CURRENT_SOURCE
SOURCE-EVIDENCE-UNKNOWN|REVIEW|SOURCE_BINDING_MISSING|EVIDENCE_SOURCE_STATE=UNKNOWN
UNKNOWN-FIELD|REVIEW|UNKNOWN_FIELD|NOT_A_FIELD=x
MISSING-FIELD|REVIEW|MISSING_REQUIRED_FIELD|CONTRACT_SOURCE=
SCOPE-INCOMPARABLE|REVIEW|SCOPE_INCOMPARABLE,!SCOPE_INFLATION|CLAIM_SCOPE=MODULE;EVIDENCE_SCOPE=FILE
SCOPE-INFLATED|FAIL|SCOPE_INFLATION|CLAIM_SCOPE=WORKSPACE;EVIDENCE_SCOPE=PACKAGE;EVIDENCE_COMMAND=cargo test --workspace
EXECUTION-CLASS-INFLATED|FAIL|EXECUTION_CLASS_INFLATION|CLAIM_EXECUTION_CLASS=REAL_RUNTIME
PLATFORM-INFLATED|FAIL|PLATFORM_SCOPE_INFLATION|CLAIM_PLATFORM=LINUX_ARM64
CLIENT-INFLATED|FAIL|CLIENT_SCOPE_INFLATION|CLAIM_CLIENT=CURSOR;CLAIM_EXECUTION_CLASS=INTEGRATION
PROSE-UNIVERSAL|REVIEW|PROSE_SCOPE_DIVERGENCE|CLAIM=Unknown hosts answer 404 on every platform
ABSENCE-REVIEW|REVIEW|ABSENCE_CLAIM_REVIEW|CLAIM_REQUIRED_EFFECT=ABSENCE_OF_DEFECT the 404 path leaks no descriptor
NO-FAILURE-AS-NO-DEFECT|FAIL|NO_FAILURE_AS_NO_DEFECT|CLAIM_REQUIRED_EFFECT=ABSENCE_OF_DEFECT the 404 path leaks no descriptor;EVIDENCE_OBSERVED_EFFECT=NO_FAILURE_OBSERVED nothing failed during the run
UNOBSERVED-EFFECT|FAIL|UNOBSERVED_EFFECT|EVIDENCE_OBSERVED_EFFECT=NOT_OBSERVED the assertion never ran
UNKNOWN-EFFECT-TOKEN|REVIEW|UNKNOWN_TOKEN|CLAIM_REQUIRED_EFFECT=no leak reported by the run
HYPOTHESIS-AS-CAUSE|FAIL|HYPOTHESIS_AS_CAUSE|EPISTEMIC_STATUS=HYPOTHESIS
RUNNER-FORM-CI|REVIEW|SCOPE_COMMAND_UNVERIFIABLE,!SCOPE_COMMAND_MISMATCH|CLAIM_SCOPE=WORKSPACE;EVIDENCE_SCOPE=WORKSPACE;EVIDENCE_COMMAND=cargo run -p xtask -- ci
WORKSPACE-TESTS|PASS|-|CLAIM_SCOPE=WORKSPACE;EVIDENCE_SCOPE=WORKSPACE;EVIDENCE_COMMAND=cargo test --workspace --tests
EXCLUDED-PACKAGE|FAIL|SCOPE_COMMAND_MISMATCH|CLAIM_SCOPE=WORKSPACE;EVIDENCE_SCOPE=WORKSPACE;EVIDENCE_COMMAND=cargo test --workspace --exclude exyonq-mod-http3
NAMED-TARGET-UNDER-WORKSPACE|FAIL|SCOPE_COMMAND_MISMATCH|CLAIM_SCOPE=WORKSPACE;EVIDENCE_SCOPE=WORKSPACE;EVIDENCE_COMMAND=cargo test --workspace --test routing_only
PACKAGE-NARROWED-WIDE-CLAIM|FAIL|SCOPE_COMMAND_MISMATCH|CLAIM_SCOPE=WORKSPACE;EVIDENCE_SCOPE=WORKSPACE;EVIDENCE_COMMAND=cargo clippy -p exyonq-core -- -D warnings
TARGET-NARROWED-WIDE-CLAIM|FAIL|SCOPE_COMMAND_MISMATCH|CLAIM_SCOPE=WORKSPACE;EVIDENCE_SCOPE=WORKSPACE;EVIDENCE_COMMAND=cargo test --lib
DISCLAIMED-PLATFORM-BOTH|REVIEW|DIMENSION_DISCLAIMED|CLAIM_EXECUTION_CLASS=BENCHMARK;EVIDENCE_EXECUTION_CLASS=BENCHMARK;CLAIM_PLATFORM=NOT_APPLICABLE;EVIDENCE_PLATFORM=NOT_APPLICABLE
DISCLAIMED-CLIENT-REAL-CLIENT|REVIEW|DIMENSION_DISCLAIMED|CLAIM_EXECUTION_CLASS=REAL_CLIENT;EVIDENCE_EXECUTION_CLASS=REAL_CLIENT;CLAIM_CLIENT=NOT_APPLICABLE;EVIDENCE_CLIENT=NOT_APPLICABLE
STATIC-ANALYSIS-NO-PLATFORM|PASS|-|CLAIM_EXECUTION_CLASS=STATIC_ANALYSIS;EVIDENCE_EXECUTION_CLASS=STATIC_ANALYSIS;CLAIM_PLATFORM=NOT_APPLICABLE;EVIDENCE_PLATFORM=NOT_APPLICABLE
FIXED-NO-CAUSAL-PATH|FAIL|CAUSAL_PATH_REQUIRED|CLAIM_STATUS=FIXED
PATH-NOT-EXECUTED|FAIL|WRONG_CAUSAL_PATH|TARGET_CAUSAL_PATH=the truncated upstream body branch;PATH_EXECUTED=NO
RESULT-PARTIAL|FAIL|PARTIAL_AS_GLOBAL|EVIDENCE_RESULT=PARTIAL
SELF-CONFIRMING-HIGH|FAIL|SELF_CONFIRMING_ORACLE|SELF_CONFIRMING_TEST_LOOP_RISK=HIGH
CONCURRENCY-NO-FORM|FAIL|CONCURRENCY_EVIDENCE_GAP|CLAIM_CONCURRENCY_CLASS=RACE;CONCURRENCY_EVIDENCE_FORM=NONE
PROSE-IN-CLASSIFIER-TAIL|REVIEW|PROSE_SCOPE_DIVERGENCE|CLAIM_REQUIRED_EFFECT=POSITIVE_OBSERVATION the status is 404 on every platform
CAUSAL-PROSE-SYNONYM|REVIEW|CAUSAL_PATH_REQUIRED|CLAIM=The latency spike stems from the descriptor table lookup
CONCURRENCY-PROSE-SYNONYM|REVIEW|CONCURRENCY_CLASS_DIVERGENCE|CLAIM=The counter update is guarded by a mutex so no torn read is possible
ROWS
}

probe_phase() {
  local base="$TMPDIR_RUN/base.cei"
  local dir="$TMPDIR_RUN/probes"
  rm -rf "$dir"
  mkdir -p "$dir"
  write_base_template "$base"

  local name expected codes overrides
  while IFS='|' read -r name expected codes overrides; do
    [[ -n "${name:-}" ]] || continue
    local target="$dir/probe-$name.cei"
    if [[ "$overrides" == "-" ]]; then
      cp "$base" "$target"
    else
      apply_mutation "$base" "$overrides" "$target"
    fi
    apply_one "$target" CLAIM_ID "PROBE-$name" > "$target.next"
    mv "$target.next" "$target"
  done < <(probe_rows)

  set +e
  local out
  out="$(/bin/bash "$CHECKER" --check "$dir" 2>/dev/null)"
  set -e

  while IFS='|' read -r name expected codes overrides; do
    [[ -n "${name:-}" ]] || continue
    cases=$((cases + 1))
    local id="PROBE-$name"
    local verdict findings probe_ok=1
    verdict="$(field_of_claim "$out" "$id" CLAIM_EVIDENCE_COMPATIBILITY)"
    findings="$(field_of_claim "$out" "$id" FINDINGS)"
    if [[ "$verdict" != "$expected" ]]; then
      fail_case "$id: expected verdict $expected, checker said ${verdict:-<none>} (${findings:-<none>})"
      probe_ok=0
    fi
    local code
    for code in $(printf '%s' "$codes" | tr ',' ' '); do
      [[ -n "$code" && "$code" != "-" ]] || continue
      note_code "$code"
      if [[ "$code" == !* ]]; then
        if [[ ",$findings," == *",${code#!},"* ]]; then
          fail_case "$id: finding ${code#!} must not appear, got ${findings:-<none>}"
          probe_ok=0
        fi
      elif [[ ",$findings," != *",$code,"* ]]; then
        fail_case "$id: expected finding $code, got ${findings:-<none>}"
        probe_ok=0
      fi
    done
    if [[ $probe_ok -eq 1 ]]; then
      ok=$((ok + 1))
      echo "CASE_RESULT=OK $id $expected ${codes}"
    fi
  done < <(probe_rows)

  # Exit-code contract. Consumers gate on the exit status, so each verdict must
  # map to the documented code: PASS 0, FAIL 1, REVIEW 3.
  local probe expect_rc rc
  for pair in "BASE:0" "RESULT-FAIL:1" "RESULT-UNKNOWN:3"; do
    probe="$dir/probe-${pair%%:*}.cei"
    expect_rc="${pair##*:}"
    set +e
    /bin/bash "$CHECKER" --check "$probe" >/dev/null 2>&1
    rc=$?
    set -e
    if [[ $rc -ne $expect_rc ]]; then
      fail_case "exit-code: ${pair%%:*} exited $rc, expected $expect_rc"
    fi
  done
}

# --- layer 2b: structural regressions ---------------------------------------

structural_phase() {
  # A parse problem must become a finding on its block, never an aborted run:
  # an abort would suppress the FAIL verdicts of every other block.
  local broken="$TMPDIR_RUN/broken.cei"
  cat > "$broken" <<'BROKEN'
BEGIN_CLAIM_EVIDENCE_BLOCK
CLAIM_ID = BROKEN-01
CLAIM_STATUS PASS
END_CLAIM_EVIDENCE_BLOCK
BROKEN
  set +e
  local broken_out broken_rc
  broken_out="$(/bin/bash "$CHECKER" --check "$broken" 2>/dev/null)"
  broken_rc=$?
  set -e
  note_code MALFORMED_BLOCK
  [[ $broken_rc -eq 1 ]] || fail_case "structural: malformed block gave exit $broken_rc, expected 1 (FAIL, not CHECKER_ERROR)"
  [[ "$broken_out" == *MALFORMED_BLOCK* ]] || fail_case "structural: malformed block did not report MALFORMED_BLOCK"

  local dup="$TMPDIR_RUN/duplicate.cei"
  cat > "$dup" <<'DUP'
BEGIN_CLAIM_EVIDENCE_BLOCK
CLAIM_ID = DUPLICATE-01
CLAIM_STATUS = REVIEW
CLAIM_STATUS = REVIEW
END_CLAIM_EVIDENCE_BLOCK
DUP
  set +e
  local dup_out
  dup_out="$(/bin/bash "$CHECKER" --check "$dup" 2>/dev/null)"
  set -e
  note_code DUPLICATE_FIELD
  [[ "$dup_out" == *DUPLICATE_FIELD* ]] || fail_case "structural: duplicated field did not report DUPLICATE_FIELD"

  # Prose that merely mentions the markers is not a block, and an explicit file
  # with no block is REVIEW, never PASS.
  local prose="$TMPDIR_RUN/prose.md"
  cat > "$prose" <<'PROSE'
The block opens with BEGIN_CLAIM_EVIDENCE_BLOCK and closes with
END_CLAIM_EVIDENCE_BLOCK, one field per line.
PROSE
  set +e
  local prose_out prose_rc
  prose_out="$(/bin/bash "$CHECKER" --check "$prose" 2>/dev/null)"
  prose_rc=$?
  set -e
  note_code NO_BLOCK_FOUND
  [[ $prose_rc -eq 3 ]] || fail_case "structural: prose mentioning the markers gave exit $prose_rc, expected 3 (REVIEW)"
  [[ "$prose_out" == *NO_BLOCK_FOUND* ]] || fail_case "structural: prose file did not report NO_BLOCK_FOUND"

  # Aggregate contract: the corpus contains proven mismatches, so a directory
  # run must surface FAIL.
  set +e
  /bin/bash "$CHECKER" --check "$FIXTURES" > "$TMPDIR_RUN/agg.out" 2>/dev/null
  local agg_rc=$?
  set -e
  [[ $agg_rc -eq 1 ]] || fail_case "aggregate: directory run exit code $agg_rc, expected 1 (CEI_CHECK=FAIL)"

  # The governance documentation describes the format, so it must survive a scan.
  local guard_doc
  guard_doc="${CEI_REPO_ROOT:-$(cd "$INTEGRITY_DIR/../.." && pwd)}/docs/governance/CLAIM_EVIDENCE_INTEGRITY_GUARD.md"
  if [[ -f "$guard_doc" ]]; then
    set +e
    /bin/bash "$CHECKER" --check "$guard_doc" > "$TMPDIR_RUN/doc.out" 2>/dev/null
    local doc_rc=$?
    set -e
    [[ $doc_rc -ne 2 ]] || fail_case "structural: scanning the guard documentation produced CHECKER_ERROR"
  fi
}

source_binding_phase() {
  # The fixtures use synthetic commit ids on purpose, so they say nothing about
  # the checkout they sit in; that leaves the git-backed path untested. This
  # probe is written inside the work tree so the checker can resolve HEAD, and
  # claims the current source while pointing at an impossible commit. Skipped
  # where git cannot answer: an environment without git is not a checker defect.
  # A mutant tree lives outside any repository, so deriving the root from this
  # script's own location would make the whole phase evaporate exactly where it
  # is needed: under mutation. The real root is handed down instead.
  local repo_root
  repo_root="${CEI_REPO_ROOT:-$(cd "$INTEGRITY_DIR/../.." && pwd)}"
  if ! git -C "$repo_root" rev-parse HEAD >/dev/null 2>&1; then
    if [[ -n "${CEI_REPO_ROOT:-}" ]]; then
      fail_case "source-binding: CEI_REPO_ROOT=$repo_root is not a git work tree"
    fi
    return 0
  fi
  local probe_dir="$repo_root/.exyonq-local/tmp"
  mkdir -p "$probe_dir"
  local probe
  probe="$(mktemp "$probe_dir/cei-source-binding-probe.XXXXXX")"
  mv "$probe" "$probe.cei"
  probe="$probe.cei"
  local base="$TMPDIR_RUN/base.cei"
  [[ -f "$base" ]] || write_base_template "$base"
  apply_mutation "$base" \
    "CLAIM_ID=PROBE-LIVE-SOURCE;CLAIM_SOURCE_STATE=CURRENT_SOURCE;EVIDENCE_SOURCE_STATE=HEAD=2222feed2222face STATE=CLEAN" \
    "$probe"
  set +e
  local probe_out probe_rc
  probe_out="$(/bin/bash "$CHECKER" --check "$probe" 2>/dev/null)"
  probe_rc=$?
  set -e
  rm -f "$probe"
  note_code STALE_EVIDENCE
  [[ $probe_rc -eq 1 ]] || fail_case "source-binding: current-source claim with a foreign evidence HEAD gave exit $probe_rc, expected 1"
  [[ "$probe_out" == *STALE_EVIDENCE* ]] || fail_case "source-binding: current-source claim with a foreign evidence HEAD did not report STALE_EVIDENCE"

  # The live clean-vs-dirty comparison. Its expectation follows the tree the
  # verifier is actually running in, so it asserts something either way rather
  # than quietly passing on a clean checkout.
  local head_sha dirty
  head_sha="$(git -C "$repo_root" rev-parse HEAD)"
  dirty=0
  [[ -n "$(git -C "$repo_root" status --porcelain 2>/dev/null)" ]] && dirty=1
  probe="$(mktemp "$probe_dir/cei-live-state-probe.XXXXXX")"
  mv "$probe" "$probe.cei"
  probe="$probe.cei"
  apply_mutation "$base" \
    "CLAIM_ID=PROBE-LIVE-STATE;CLAIM_SOURCE_STATE=CURRENT_SOURCE STATE=CLEAN;EVIDENCE_SOURCE_STATE=HEAD=$head_sha STATE=CLEAN;SOURCE_BINDING=REAL" \
    "$probe"
  set +e
  probe_out="$(/bin/bash "$CHECKER" --check "$probe" 2>/dev/null)"
  set -e
  rm -f "$probe"
  # A well-formed but fabricated commit id. Only reachable inside a repository,
  # which is why it lives here rather than in the probe layer.
  probe="$(mktemp "$probe_dir/cei-unresolvable-probe.XXXXXX")"
  mv "$probe" "$probe.cei"
  probe="$probe.cei"
  apply_mutation "$base" "CLAIM_ID=PROBE-UNRESOLVABLE;SOURCE_BINDING=REAL" "$probe"
  set +e
  local unres_out
  unres_out="$(/bin/bash "$CHECKER" --check "$probe" 2>/dev/null)"
  set -e
  rm -f "$probe"
  note_code UNRESOLVABLE_SOURCE_BINDING
  [[ "$unres_out" == *UNRESOLVABLE_SOURCE_BINDING* ]] || fail_case "source-binding: a fabricated commit id was accepted as a source binding"

  note_code SOURCE_STATE_MISMATCH
  if [[ $dirty -eq 1 ]]; then
    [[ "$probe_out" == *SOURCE_STATE_MISMATCH* ]] || fail_case "source-binding: clean-source claim in a dirty work tree did not report SOURCE_STATE_MISMATCH"
  else
    [[ "$probe_out" != *SOURCE_STATE_MISMATCH* ]] || fail_case "source-binding: clean-source claim in a clean work tree wrongly reported SOURCE_STATE_MISMATCH"
  fi
}

# --- layer 2c: gate coverage -------------------------------------------------

coverage_phase() {
  # Every finding code the engine can emit must be asserted on by something
  # above. An unexercised gate is a gate that can be deleted unnoticed, which
  # is exactly the defect this file exists to prevent in the product.
  local emitted asserted missing
  emitted="$(python3 - "$ENGINE" <<'PY'
import re, sys

text = open(sys.argv[1], encoding="utf-8").read()
codes = set()
# First argument of every Finding(...) construction; it is a literal or a
# ternary over literals, never an expression containing a comma.
for arg in re.findall(r"Finding\(\s*([^,]+),", text, re.S):
    codes.update(re.findall(r'"([A-Z][A-Z_]{3,})"', arg))
# Codes reached through the epistemic table, where Finding() sees a variable.
for code in re.findall(r'\(\s*"(?:FAIL|REVIEW)",\s*"([A-Z_]+)"\)', text):
    codes.add(code)
print("\n".join(sorted(codes)))
PY
)"
  asserted="$(printf '%s\n' "${ASSERTED_CODES[@]}" | sort -u)"
  missing="$(comm -23 <(printf '%s\n' "$emitted") <(printf '%s\n' "$asserted"))"
  if [[ -n "$missing" ]]; then
    local code
    while read -r code; do
      [[ -n "$code" ]] || continue
      fail_case "coverage: finding code $code is never exercised by a case or probe"
    done <<< "$missing"
  fi
}

# --- layer 3: mutants --------------------------------------------------------

mutate_engine() { # dest old new
  python3 - "$ENGINE" "$1" "$2" "$3" <<'PY'
import sys
src, dst, old, new = sys.argv[1:5]
text = open(src, encoding="utf-8").read()
if old not in text:
    sys.stderr.write("anchor not found: %r\n" % old)
    sys.exit(9)
open(dst, "w", encoding="utf-8").write(text.replace(old, new, 1))
PY
}

# One gate per row: NAME | ANCHOR | REPLACEMENT. Switching a single gate off
# must be caught. If a row's anchor stops matching after a refactor the mutant
# harness fails loudly rather than silently testing nothing.
gate_mutants() {
  cat <<'ROWS'
result-not-pass|        if result[0] in {"FAIL", "BLOCKED", "NOT_RUN"}:|        if False:
epistemic-block|    if material and epistemic:|    if False and material and epistemic:
oracle-derived|    if material and derived:|    if False and material and derived:
dimension-disclaimed|        if material and ev_set != {"NOT_APPLICABLE"}:|        if False and material:
source-binding-missing|    if material and not says_current and "HEAD" not in claim and "TREE" not in claim:|    if False:
source-state-mismatch|    if claim.get("STATE", "").upper() == "CLEAN" and evidence.get("STATE", "").upper() == "DIRTY":|    if False:
declared-honored|    elif rank[declared[0]] < rank[computed]:|    elif False:
declared-inflated|    if rank[declared[0]] > rank[computed]:|    if False:
causal-disclaimed|        if material and proof in {"STATUS_CODE_ONLY", "SAME_OUTPUT"}:|        if False:
effect-disclaimed|    if "NOT_APPLICABLE" in (req_tok, obs_tok):|    if False:
concurrency-prose|    if concurrency in {"", "NONE"} and CONCURRENCY_WORDS.search(prose):|    if False:
self-confirming-medium|        elif risk[0] == "MEDIUM":|        elif False:
unknown-status-material|            material = True\n\n    if material:|            material = False\n\n    if material:
runtime-dimension|    if platform == {"NOT_APPLICABLE"} and ran and classes and classes != {"STATIC_ANALYSIS"}:|    if False:
runtime-client-dimension|    if "REAL_CLIENT" in classes and client == {"NOT_APPLICABLE"}:|    if False:
unresolvable-binding|            if not git_object_exists(Path(block.source).resolve().parent, sha):|            if False:
named-target-narrowing|            or NAMED_TARGET_CMD.search(command)|            or False
target-narrowing|            or (TARGET_NARROWING_CMD.search(command) and not wide_cmd)|            or False
package-narrowing|            or (PACKAGE_NARROWING_CMD.search(command) and not wide_cmd and not runner)|            or False
causal-path-not-executed|    if executed[0] == "NO":|    if False:
result-partial|        elif result[0] == "PARTIAL":|        elif False:
self-confirming-high|        if risk[0] == "HIGH":|        if False:
concurrency-evidence-gap|    if not forms & SUFFICIENT_CONCURRENCY_FORMS:|    if False:
requires-causal-path|    is_root_cause = status in REQUIRES_CAUSAL_PATH or kind_raw == "ROOT_CAUSE"|    is_root_cause = status == "ROOT_CAUSE" or kind_raw == "ROOT_CAUSE"
prose-all-fields|        free_text_of(k, block.get(k) or "") for k in block.fields|        free_text_of(k, block.get(k) or "") for k in ("CLAIM", "EVIDENCE")
ROWS
}

build_mutant_tree() { # dir engine_source
  local dir="$1" engine="$2"
  mkdir -p "$dir/lib" "$dir/tests" "$dir/fixtures"
  cp "$CHECKER" "$dir/"
  cp -R "$FIXTURES" "$dir/fixtures/"
  cp "${BASH_SOURCE[0]}" "$dir/tests/"
  cp "$engine" "$dir/lib/claim_evidence_check.py"
}

mutant_phase() {
  # Whole-verdict mutants against the full verifier: a checker that answers the
  # same thing for every input must not be able to satisfy this file.
  local mode mdir verdict mrc
  for mode in ALWAYS_FAIL ALWAYS_PASS; do
    mdir="$TMPDIR_RUN/mutant-$mode"
    verdict="FAIL"; [[ "$mode" == "ALWAYS_PASS" ]] && verdict="PASS"
    awk -v v="$verdict" '
      /^if __name__ == "__main__":/ && !done {
        print "def verdict_for(findings):"
        print "    return \"" v "\""
        print ""
        done = 1
      }
      { print }
    ' "$ENGINE" > "$TMPDIR_RUN/engine-$mode.py"
    build_mutant_tree "$mdir" "$TMPDIR_RUN/engine-$mode.py"
    set +e
    CEI_MUTANT_RUN=1 CEI_REPO_ROOT="$REPO_ROOT" \
      /bin/bash "$mdir/tests/$(basename "${BASH_SOURCE[0]}")" > "$mdir/out.txt" 2>&1
    mrc=$?
    set -e
    # Exit 1 specifically: caught by a reported deviation, not by the harness
    # crashing for an unrelated reason.
    [[ $mrc -eq 1 ]] || fail_case "meta: the $mode mutant returned $mrc, expected 1 (deviations reported)"
  done

  # Per-gate mutants against the probe layer, which is where a single deleted
  # gate shows. Each must be caught.
  local name anchor replacement
  while IFS='|' read -r name anchor replacement; do
    [[ -n "${name:-}" ]] || continue
    mdir="$TMPDIR_RUN/gate-$name"
    mkdir -p "$mdir"
    anchor="$(printf '%b' "$anchor")"
    replacement="$(printf '%b' "$replacement")"
    if ! mutate_engine "$TMPDIR_RUN/engine-$name.py" "$anchor" "$replacement"; then
      fail_case "meta: gate mutant $name could not be built; the anchor no longer matches the engine"
      continue
    fi
    build_mutant_tree "$mdir" "$TMPDIR_RUN/engine-$name.py"
    set +e
    CEI_MUTANT_RUN=1 CEI_PROBES_ONLY=1 CEI_REPO_ROOT="$REPO_ROOT" \
      /bin/bash "$mdir/tests/$(basename "${BASH_SOURCE[0]}")" > "$mdir/out.txt" 2>&1
    mrc=$?
    set -e
    if [[ $mrc -ne 1 ]]; then
      fail_case "meta: switching off gate '$name' was not detected (verifier returned $mrc, expected 1)"
    else
      echo "MUTANT_DETECTED=$name"
    fi
  done < <(gate_mutants)
}

# --- run ---------------------------------------------------------------------

if [[ "$PROBES_ONLY" == "1" ]]; then
  # The fast layer used by the per-gate mutants. The live-source probe joins it
  # because the git-backed gates have no other test, and skipping them here
  # would leave them mutation-untested.
  probe_phase
  source_binding_phase
else
  owner_cases
  probe_phase
  structural_phase
  source_binding_phase
  coverage_phase
  if [[ "${CEI_MUTANT_RUN:-0}" != "1" ]]; then
    mutant_phase
  fi
fi

echo "RULE_ID=CLAIM-EVIDENCE-INTEGRITY"
echo "ADVERSARIAL_CASES=$cases"
echo "ADVERSARIAL_PASS=$ok"
echo "ADVERSARIAL_FAIL=$bad"
if [[ $bad -ne 0 ]]; then
  for m in "${FAILURES[@]}"; do echo "ERROR: $m" >&2; done
  echo "CEI_SELFTEST=FAIL"
  exit 1
fi
echo "CEI_SELFTEST=PASS"
exit 0
