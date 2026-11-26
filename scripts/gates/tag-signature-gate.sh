#!/usr/bin/env bash
# Release-tag authenticity: a signed annotated tag must be verified by the
# project's own verifier, every time, instead of by a command someone has to
# remember.
#
# `scripts/release/verify-tag-signature.sh` already enforces the contract
# (annotated tag, authorized principal, exact commit, version match, SSH
# signature against the tracked allowed-signers file), but nothing called it:
# the v0.9.2 tag was verified by hand on 2026-11-20. This gate is the repeatable
# entry point:
#
#   * on an untagged working tree it prints RESULT=NOT_APPLICABLE (exit 0) and
#     never a pass, so `cargo run -p xtask -- ci` can always run it;
#   * when HEAD is a release tag, or a tag is supplied, it verifies the
#     signature and fails closed on any deviation;
#   * `--selftest` proves it is not vacuous using a real `ssh-keygen` key and
#     real git tags, covering signed, unsigned, lightweight, unauthorized
#     principal, wrong commit and untagged cases.
#
# Usage:
#   bash scripts/gates/tag-signature-gate.sh                    # tag from HEAD
#   bash scripts/gates/tag-signature-gate.sh --tag v0.9.3
#   RELEASE_TAG=v0.9.3 bash scripts/gates/tag-signature-gate.sh
#   bash scripts/gates/tag-signature-gate.sh --expected-commit <sha40>
#   bash scripts/gates/tag-signature-gate.sh --selftest
set -uo pipefail
LC_ALL=C
export LC_ALL

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VERIFIER="$ROOT/scripts/release/verify-tag-signature.sh"
DEFAULT_SIGNERS="$ROOT/security/signing/exyonq-release-tag-signers"

usage() {
  cat <<'EOF'
Usage:
  tag-signature-gate.sh [--tag TAG] [--expected-commit SHA40]
                        [--repo PATH] [--allowed-signers PATH]
  tag-signature-gate.sh --selftest

Verifies the SSH signature of the release tag with
scripts/release/verify-tag-signature.sh --repo . --tag TAG
  --expected-version <tag without leading v>
  --expected-commit <peeled commit, or --expected-commit>
  --allowed-signers <tracked allowed-signers file>

Without --tag, the tag that points at HEAD is used (RELEASE_TAG is accepted as
an environment override). An untagged tree is RESULT=NOT_APPLICABLE, exit 0,
never a pass: the gate exists to be run on every CI pass, including branches.
EOF
}

die_usage() {
  echo "ERROR: $1" >&2
  usage >&2
  exit 2
}

# verify_tag <repo> <tag> <expected-commit> <allowed-signers>
# Runs the project verifier and echoes its output. Returns the verifier's rc.
verify_tag() {
  local repo="$1" tag="$2" commit="$3" signers="$4" version
  version="${tag#v}"
  bash "$VERIFIER" \
    --repo "$repo" \
    --tag "$tag" \
    --expected-version "$version" \
    --expected-commit "$commit" \
    --allowed-signers "$signers" 2>&1
}

TAG="${RELEASE_TAG:-}"
EXPECTED_COMMIT=""
REPO="$ROOT"
SIGNERS="$DEFAULT_SIGNERS"
SELFTEST=0
SELFTEST_TMP=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag) [[ $# -ge 2 ]] || die_usage "--tag needs a value"; TAG="$2"; shift 2 ;;
    --expected-commit)
      [[ $# -ge 2 ]] || die_usage "--expected-commit needs a value"
      EXPECTED_COMMIT="$2"
      shift 2
      ;;
    --repo) [[ $# -ge 2 ]] || die_usage "--repo needs a value"; REPO="$2"; shift 2 ;;
    --allowed-signers)
      [[ $# -ge 2 ]] || die_usage "--allowed-signers needs a value"
      SIGNERS="$2"
      shift 2
      ;;
    --selftest) SELFTEST=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die_usage "unknown argument: $1" ;;
  esac
done

selftest() {
  local tmp rc failures=0
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/tag-signature-gate.XXXXXX")" || {
    echo "RESULT=FAIL TAG_SIGNATURE_GATE_SELFTEST (mktemp)"
    return 2
  }
  SELFTEST_TMP="$tmp"
  trap 'rm -rf "${SELFTEST_TMP:-}"' EXIT

  local repo="$tmp/repo" signers="$tmp/allowed_signers"
  mkdir -p "$repo"
  git -C "$repo" init -q
  git -C "$repo" config user.name "ExyonQ Release"
  git -C "$repo" config user.email "exyonq-release"
  git -C "$repo" config commit.gpgsign false
  printf 'selftest\n' >"$repo/file.txt"
  git -C "$repo" add file.txt
  git -C "$repo" commit -q -m "selftest base"
  ssh-keygen -q -t ed25519 -N '' -f "$tmp/authorized" >/dev/null 2>&1 || {
    echo "RESULT=FAIL TAG_SIGNATURE_GATE_SELFTEST (ssh-keygen)"
    return 2
  }
  ssh-keygen -q -t ed25519 -N '' -f "$tmp/stranger" >/dev/null 2>&1 || {
    echo "RESULT=FAIL TAG_SIGNATURE_GATE_SELFTEST (ssh-keygen)"
    return 2
  }
  {
    printf 'exyonq-release '
    cat "$tmp/authorized.pub"
  } >"$signers"

  local base_commit
  base_commit="$(git -C "$repo" rev-parse HEAD)"

  # run_gate <expected rc> <name> [args...]
  run_gate() {
    local want="$1" name="$2" out got
    shift 2
    out="$(bash "${BASH_SOURCE[0]}" --repo "$repo" "$@" 2>&1)"
    got=$?
    if [[ "$got" -eq "$want" ]]; then
      echo "PASS selftest $name (rc=$got)"
      return 0
    fi
    echo "FAIL selftest $name expected_rc=$want got=$got"
    printf '%s\n' "$out" | sed 's/^/  /'
    failures=$((failures + 1))
  }

  expect_output() { # expect_output <name> <pattern> <args...>
    local name="$1" pattern="$2" out
    shift 2
    out="$(bash "${BASH_SOURCE[0]}" --repo "$repo" "$@" 2>&1)"
    if printf '%s\n' "$out" | grep -q "$pattern"; then
      echo "PASS selftest $name"
      return 0
    fi
    echo "FAIL selftest $name (missing: $pattern)"
    printf '%s\n' "$out" | sed 's/^/  /'
    failures=$((failures + 1))
  }

  # Untagged tree: NOT_APPLICABLE, never a pass.
  expect_output "untagged tree is NOT_APPLICABLE" "RESULT=NOT_APPLICABLE TAG_SIGNATURE_GATE"

  # Signed annotated tag by the authorized principal.
  git -C "$repo" -c gpg.format=ssh -c user.signingkey="$tmp/authorized" \
    tag -s -m "release" v9.9.9
  run_gate 0 "signed annotated tag" --tag v9.9.9 --allowed-signers "$signers"

  # Same tag, wrong expected commit.
  printf 'second\n' >"$repo/file.txt"
  git -C "$repo" commit -q -am "second"
  run_gate 1 "wrong expected commit" --tag v9.9.9 \
    --expected-commit "$(git -C "$repo" rev-parse HEAD)" --allowed-signers "$signers"

  # Signed by a key the allowed-signers file does not authorize.
  git -C "$repo" tag -d v9.9.9 >/dev/null
  git -C "$repo" -c gpg.format=ssh -c user.signingkey="$tmp/stranger" \
    tag -s -m "release" v9.9.9
  run_gate 1 "unauthorized signer" --tag v9.9.9 --allowed-signers "$signers"

  # Annotated but unsigned.
  git -C "$repo" tag -d v9.9.9 >/dev/null
  git -C "$repo" tag -a -m "release" v9.9.9
  run_gate 1 "unsigned annotated tag" --tag v9.9.9 --allowed-signers "$signers"

  # Lightweight tag.
  git -C "$repo" tag -d v9.9.9 >/dev/null
  git -C "$repo" tag v9.9.9
  run_gate 1 "lightweight tag" --tag v9.9.9 --allowed-signers "$signers"

  # Empty allowed-signers (NOT_YET_PROVISIONED) is not a pass.
  git -C "$repo" tag -d v9.9.9 >/dev/null
  git -C "$repo" -c gpg.format=ssh -c user.signingkey="$tmp/authorized" \
    tag -s -m "release" v9.9.9
  : >"$tmp/empty_signers"
  run_gate 1 "empty allowed-signers" --tag v9.9.9 --allowed-signers "$tmp/empty_signers"

  if [[ "$failures" -ne 0 ]]; then
    echo "RESULT=FAIL TAG_SIGNATURE_GATE_SELFTEST failures=$failures"
    return 1
  fi
  echo "RESULT=PASS TAG_SIGNATURE_GATE_SELFTEST"
  return 0
}

if [[ "$SELFTEST" == "1" ]]; then
  selftest
  exit $?
fi

[[ -f "$VERIFIER" ]] || {
  echo "FAIL missing verifier: $VERIFIER"
  echo "RESULT=FAIL TAG_SIGNATURE_GATE"
  exit 2
}
[[ -f "$SIGNERS" ]] || {
  echo "FAIL missing allowed-signers: $SIGNERS"
  echo "RESULT=FAIL TAG_SIGNATURE_GATE"
  exit 2
}

head="$(git -C "$REPO" rev-parse HEAD 2>/dev/null || true)"
if [[ -z "$head" ]]; then
  echo "FAIL not a git repository: $REPO"
  echo "RESULT=FAIL TAG_SIGNATURE_GATE"
  exit 2
fi

TAG_AT_HEAD=0
if [[ -z "$TAG" ]]; then
  TAG="$(git -C "$REPO" describe --tags --exact-match HEAD 2>/dev/null || true)"
  [[ -n "$TAG" ]] && TAG_AT_HEAD=1
fi

if [[ -z "$TAG" ]]; then
  echo "RESULT=NOT_APPLICABLE TAG_SIGNATURE_GATE head=${head:0:7} (not a tag; pass --tag or RELEASE_TAG to enforce)"
  exit 0
fi

if [[ -z "$EXPECTED_COMMIT" ]]; then
  if [[ "$TAG_AT_HEAD" == "1" ]]; then
    # The tag was found on HEAD, so HEAD is the commit it must point at; this is
    # what makes the check non-circular.
    EXPECTED_COMMIT="$head"
  else
    EXPECTED_COMMIT="$(git -C "$REPO" rev-parse "${TAG}^{}" 2>/dev/null || true)"
  fi
fi
if [[ -z "$EXPECTED_COMMIT" ]]; then
  echo "FAIL cannot resolve tag: $TAG"
  echo "RESULT=FAIL TAG_SIGNATURE_GATE tag=$TAG"
  exit 1
fi

if ! out="$(verify_tag "$REPO" "$TAG" "$EXPECTED_COMMIT" "$SIGNERS")"; then
  printf '%s\n' "$out"
  echo "RESULT=FAIL TAG_SIGNATURE_GATE tag=$TAG version=${TAG#v} commit=$EXPECTED_COMMIT"
  exit 1
fi

printf '%s\n' "$out"
echo "RESULT=PASS TAG_SIGNATURE_GATE tag=$TAG version=${TAG#v} commit=$EXPECTED_COMMIT"
exit 0
