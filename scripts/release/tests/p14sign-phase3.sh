#!/usr/bin/env bash
# P14SIGN Phase 3 — TEST_ONLY crypto fixtures + negative-path matrix.
# Does not persist private keys. Does not commit/push/publish.
set -euo pipefail
LC_ALL=C
export LC_ALL

# Never enable shell xtrace (would leak passphrases).
set +x

SCRIPT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
TESTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RELEASE_SCRIPTS="$(cd "$TESTS_DIR/.." && pwd)"
REPO_ROOT="$(cd "$RELEASE_SCRIPTS/../.." && pwd)"
# shellcheck source=scripts/release/lib/p14sign-common.sh
source "$RELEASE_SCRIPTS/lib/p14sign-common.sh"

KEEP_PUBLIC_EVIDENCE=0
WORK_ROOT=""
PASS_FILE=""
RESULTS_JSON=""
PRIVATE_KEY_PATHS=()
COSIGN_PASS_FILE=""
SUMMARY_LINES=()

usage() {
  cat <<'EOF'
Usage:
  p14sign-phase3.sh [--keep-public-evidence] [--help]

Creates an ephemeral temp dir under .exyonq-local/tmp/, generates TEST_ONLY
keys, runs positive SSH/Cosign/checksum fixtures and the N01–N40 negative
matrix, then shreds private keys (PASS and FAIL).

Options:
  --keep-public-evidence   Keep redacted logs, public keys, hashes under the
                           work dir (private keys still deleted).
  --help                   Show this help.

Forbidden:
  --keep-private-keys      (not supported; refused)
EOF
}

die() { echo "ERROR: $*" >&2; exit 1; }

redact_log() {
  # Strip obvious secret-looking lines from a log file copy.
  local src="$1" dst="$2"
  if [[ ! -f "$src" ]]; then
    : >"$dst"
    return 0
  fi
  # Drop lines that look like PEM private material or passphrase assignment.
  LC_ALL=C grep -vE \
    'BEGIN (ENCRYPTED )?PRIVATE KEY|BEGIN OPENSSH PRIVATE KEY|BEGIN RSA PRIVATE KEY|BEGIN EC PRIVATE KEY|BEGIN PGP PRIVATE KEY|COSIGN_PASSWORD=|passphrase' \
    "$src" >"$dst" 2>/dev/null || true
}

classify_stderr() {
  local log="$1"
  if [[ ! -f "$log" ]]; then
    echo "EMPTY"
    return 0
  fi
  if LC_ALL=C grep -qiE 'private key|looks like a private key' "$log"; then
    echo "PRIVATE_KEY_GUARD"; return 0
  fi
  if LC_ALL=C grep -qiE 'hash mismatch|checksum|SHA256|INTEGRITY|malformed|duplicate|forbidden entry|illegal path|symlink|missing file|undeclared|manifest' "$log"; then
    echo "INTEGRITY_OR_MANIFEST"; return 0
  fi
  if LC_ALL=C grep -qiE 'tag must|lightweight|annotated|verify-tag|allowed-signers|commit mismatch|legacy|latest forbidden|test-fixture' "$log"; then
    echo "TAG_OR_VERSION_BINDING"; return 0
  fi
  if LC_ALL=C grep -qiE 'OCI|digest|latest forbidden|tag-only|mutable tag' "$log"; then
    echo "OCI_PRECONDITION"; return 0
  fi
  if LC_ALL=C grep -qiE 'cosign is not installed|COSIGN' "$log"; then
    echo "TOOLING_MISSING"; return 0
  fi
  if LC_ALL=C grep -qiE 'dirty release tree|repo not found|not found|does not exist' "$log"; then
    echo "MISSING_OR_DIRTY"; return 0
  fi
  if LC_ALL=C grep -qiE 'error|ERROR|failed|FAIL' "$log"; then
    echo "GENERIC_ERROR"; return 0
  fi
  echo "UNCLASSIFIED"
}

secret_leak_in_file() {
  local f="$1"
  [[ -f "$f" ]] || return 1
  if LC_ALL=C grep -qE 'BEGIN (ENCRYPTED )?PRIVATE KEY|BEGIN OPENSSH PRIVATE KEY|COSIGN_PASSWORD=' "$f" 2>/dev/null; then
    return 0
  fi
  if [[ -n "${COSIGN_PASS_FILE:-}" && -f "$COSIGN_PASS_FILE" ]]; then
    local pw
    pw="$(cat "$COSIGN_PASS_FILE" 2>/dev/null || true)"
    if [[ -n "$pw" ]] && LC_ALL=C grep -Fq -- "$pw" "$f" 2>/dev/null; then
      return 0
    fi
  fi
  return 1
}

cleanup_private_keys() {
  local p
  for p in "${PRIVATE_KEY_PATHS[@]:-}"; do
    [[ -n "$p" ]] || continue
    if [[ -f "$p" ]]; then
      if command -v shred >/dev/null 2>&1; then
        shred -u "$p" 2>/dev/null || rm -f "$p"
      else
        # macOS: overwrite then unlink
        dd if=/dev/urandom of="$p" bs=1024 count=4 conv=notrunc 2>/dev/null || true
        rm -f "$p"
      fi
    fi
  done
  if [[ -n "${COSIGN_PASS_FILE:-}" && -f "$COSIGN_PASS_FILE" ]]; then
    rm -f "$COSIGN_PASS_FILE"
  fi
  unset COSIGN_PASSWORD || true
}

finalize_evidence() {
  local status="$1"
  cleanup_private_keys
  if [[ -n "$WORK_ROOT" && -d "$WORK_ROOT" ]]; then
    # Remove any remaining private-looking files under work root.
    local f
    while IFS= read -r -d '' f; do
      if p14sign_looks_private_key "$f"; then
        rm -f "$f"
      fi
    done < <(find "$WORK_ROOT" -type f -print0 2>/dev/null || true)
    if [[ "$KEEP_PUBLIC_EVIDENCE" != "1" ]]; then
      # Keep only summary + redacted logs if present; drop keys dir contents.
      rm -rf "$WORK_ROOT/ssh" "$WORK_ROOT/cosign-keys" "$WORK_ROOT/pass" 2>/dev/null || true
    else
      rm -rf "$WORK_ROOT/ssh/exyonq-test-signing" \
             "$WORK_ROOT/cosign-keys/cosign.key" \
             "$WORK_ROOT/pass" 2>/dev/null || true
    fi
  fi
  echo "CLEANUP_STATUS=$status" >&2
}

on_exit() {
  local ec=$?
  finalize_evidence "EXIT_$ec"
  exit "$ec"
}
trap on_exit EXIT

record_result() {
  local id="$1" expected="$2" actual_ec="$3" actual="$4" classification="$5" leak="$6"
  SUMMARY_LINES+=("$id|$expected|$actual_ec|$actual|$classification|$leak")
  printf '%s\n' \
    "TEST_ID=$id" \
    "EXPECTED_RESULT=$expected" \
    "ACTUAL_EXIT_CODE=$actual_ec" \
    "ACTUAL_RESULT=$actual" \
    "STDERR_CLASSIFICATION=$classification" \
    "SECRET_LEAK_DETECTED=$leak" >>"$RESULTS_JSON"
  if [[ "$expected" == "REJECT" && "$actual" == "REJECT" && "$leak" == "NO" ]]; then
    echo "PASS $id" >>"$PASS_FILE"
  elif [[ "$expected" == "ACCEPT" && "$actual" == "ACCEPT" && "$leak" == "NO" ]]; then
    echo "PASS $id" >>"$PASS_FILE"
  else
    echo "FAIL $id expected=$expected actual=$actual ec=$actual_ec leak=$leak class=$classification" >>"$PASS_FILE"
  fi
}

run_expect_reject() {
  local id="$1"
  shift
  local log="$WORK_ROOT/logs/${id}.stderr"
  local out="$WORK_ROOT/logs/${id}.stdout"
  mkdir -p "$WORK_ROOT/logs"
  local ec=0
  set +e
  "$@" >"$out" 2>"$log"
  ec=$?
  set -e
  local actual="ACCEPT"
  [[ "$ec" -ne 0 ]] && actual="REJECT"
  local class leak="NO"
  class="$(classify_stderr "$log")"
  if secret_leak_in_file "$log" || secret_leak_in_file "$out"; then
    leak="YES"
  fi
  redact_log "$log" "$WORK_ROOT/logs/${id}.stderr.redacted"
  record_result "$id" "REJECT" "$ec" "$actual" "$class" "$leak"
}

run_expect_accept() {
  local id="$1"
  shift
  local log="$WORK_ROOT/logs/${id}.stderr"
  local out="$WORK_ROOT/logs/${id}.stdout"
  mkdir -p "$WORK_ROOT/logs"
  local ec=0
  set +e
  "$@" >"$out" 2>"$log"
  ec=$?
  set -e
  local actual="REJECT"
  [[ "$ec" -eq 0 ]] && actual="ACCEPT"
  local class leak="NO"
  class="$(classify_stderr "$log")"
  if secret_leak_in_file "$log" || secret_leak_in_file "$out"; then
    leak="YES"
  fi
  redact_log "$log" "$WORK_ROOT/logs/${id}.stderr.redacted"
  record_result "$id" "ACCEPT" "$ec" "$actual" "$class" "$leak"
}

clone_bundle() {
  local src="$1" dst="$2"
  rm -rf "$dst"
  mkdir -p "$dst"
  # Prefer ditto/cp -R preserving structure; no symlinks expected.
  cp -R "$src/." "$dst/"
  # Ensure no accidental private keys copied into case dirs from elsewhere.
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --keep-public-evidence) KEEP_PUBLIC_EVIDENCE=1; shift ;;
    --keep-private-keys) die "--keep-private-keys is forbidden" ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

# Prefer system python over pyenv shims for harness hermeticity.
export P14SIGN_PYTHON3="${P14SIGN_PYTHON3:-/usr/bin/python3}"
p14sign_require_python3

# --- Cosign on PATH (local bootstrap preferred) ---
COSIGN_TOOL_DIR="$REPO_ROOT/.exyonq-local/tools/cosign"
if [[ -x "$COSIGN_TOOL_DIR/cosign" ]]; then
  export PATH="$COSIGN_TOOL_DIR:/usr/bin:/opt/homebrew/bin:$PATH"
  export COSIGN_BIN="$COSIGN_TOOL_DIR/cosign"
elif command -v cosign >/dev/null 2>&1; then
  export PATH="/usr/bin:/opt/homebrew/bin:$PATH"
  export COSIGN_BIN="$(command -v cosign)"
else
  die "cosign missing; bootstrap under .exyonq-local/tools/cosign/ first"
fi
export PATH="/usr/bin:/opt/homebrew/bin:$COSIGN_TOOL_DIR:$PATH"

TS="$(date -u +%Y%m%dT%H%M%SZ)"
WORK_ROOT="$(mktemp -d "$REPO_ROOT/.exyonq-local/tmp/p14sign-phase3-${TS}-XXXXXX")"
PASS_FILE="$WORK_ROOT/PASS_FAIL.txt"
RESULTS_JSON="$WORK_ROOT/results.machine.txt"
: >"$PASS_FILE"
: >"$RESULTS_JSON"

echo "WORK_ROOT=$WORK_ROOT"
echo "KEY_CLASS=TEST_ONLY"
echo "KEY_PURPOSE=P14SIGN_PHASE3_FIXTURE"
echo "KEY_PERSISTED=NO"
echo "KEY_COMMITTED=NO"
echo "KEY_UPLOADED=NO"
echo "KEY_REUSED=NO"

# Mark work root
cat >"$WORK_ROOT/TEST_ONLY_MARKER.txt" <<'EOF'
TEST_ONLY
NON_PRODUCTION
DO_NOT_PUBLISH
KEY_CLASS=TEST_ONLY
KEY_PURPOSE=P14SIGN_PHASE3_FIXTURE
EOF

# ========== Part A: SSH fixture ==========
SSH_DIR="$WORK_ROOT/ssh"
mkdir -p "$SSH_DIR"
ssh-keygen -t ed25519 -C "ExyonQ P14SIGN TEST_ONLY" -f "$SSH_DIR/exyonq-test-signing" -N '' >/dev/null
PRIVATE_KEY_PATHS+=("$SSH_DIR/exyonq-test-signing")
printf '%s\n' "TEST_ONLY" "NON_PRODUCTION" "DO_NOT_PUBLISH" >"$SSH_DIR/KEY_MARKERS.txt"

# Wrong key for negatives
ssh-keygen -t ed25519 -C "ExyonQ P14SIGN WRONG KEY" -f "$SSH_DIR/wrong-signing" -N '' >/dev/null
PRIVATE_KEY_PATHS+=("$SSH_DIR/wrong-signing")

REPO_FIX="$WORK_ROOT/git-fixture"
mkdir -p "$REPO_FIX"
git -C "$REPO_FIX" init -q
git -C "$REPO_FIX" config user.email "p14sign-test@exyonq.local"
git -C "$REPO_FIX" config user.name "P14SIGN TEST_ONLY"
echo "fixture" >"$REPO_FIX/README.md"
git -C "$REPO_FIX" add README.md
git -C "$REPO_FIX" commit -qm "p14sign test-only fixture"
COMMIT40="$(git -C "$REPO_FIX" rev-parse HEAD | tr 'A-F' 'a-f')"
TAG_NAME="p14sign-test-only-v0.4.0"
VERSION_FIX="0.4.0"

ALLOWED="$SSH_DIR/allowed_signers"
# OpenSSH/git allowedSignersFile: principal + optional namespaces + keytype + key
PUB_CORE="$(awk '{print $1 " " $2}' "$SSH_DIR/exyonq-test-signing.pub")"
printf '%s namespaces="git" %s\n' "p14sign-test@exyonq.local" "$PUB_CORE" >"$ALLOWED"

git -C "$REPO_FIX" \
  -c gpg.format=ssh \
  -c user.signingkey="$SSH_DIR/exyonq-test-signing" \
  tag -s -m "TEST_ONLY annotated tag DO_NOT_PUBLISH" "$TAG_NAME"

run_expect_accept POS_SSH_VERIFY \
  bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
    --repo "$REPO_FIX" \
    --tag "$TAG_NAME" \
    --expected-version "$VERSION_FIX" \
    --expected-commit "$COMMIT40" \
    --allowed-signers "$ALLOWED" \
    --test-fixture

# SSH negatives
WRONG_ALLOWED="$SSH_DIR/allowed_signers_wrong"
WRONG_PUB_CORE="$(awk '{print $1 " " $2}' "$SSH_DIR/wrong-signing.pub")"
printf '%s namespaces="git" %s\n' "p14sign-test@exyonq.local" "$WRONG_PUB_CORE" >"$WRONG_ALLOWED"
run_expect_reject SSH_NEG_WRONG_PUBKEY \
  bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
    --repo "$REPO_FIX" --tag "$TAG_NAME" --expected-version "$VERSION_FIX" \
    --expected-commit "$COMMIT40" --allowed-signers "$WRONG_ALLOWED" --test-fixture

EMPTY_ALLOWED="$SSH_DIR/allowed_signers_empty"
printf '# empty\n' >"$EMPTY_ALLOWED"
run_expect_reject SSH_NEG_EMPTY_ALLOWED \
  bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
    --repo "$REPO_FIX" --tag "$TAG_NAME" --expected-version "$VERSION_FIX" \
    --expected-commit "$COMMIT40" --allowed-signers "$EMPTY_ALLOWED" --test-fixture

BAD_IDENT="$SSH_DIR/allowed_signers_bad_ident"
# Wrong principal (does not match tagger email) with correct key material shape.
printf '%s namespaces="git" %s\n' "unauthorized@example.invalid" "$PUB_CORE" >"$BAD_IDENT"
run_expect_reject SSH_NEG_UNAUTHORIZED_IDENTITY \
  bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
    --repo "$REPO_FIX" --tag "$TAG_NAME" --expected-version "$VERSION_FIX" \
    --expected-commit "$COMMIT40" --allowed-signers "$BAD_IDENT" --test-fixture

# Lightweight tag
git -C "$REPO_FIX" tag "p14sign-test-only-v0.4.0-light" HEAD
run_expect_reject SSH_NEG_LIGHTWEIGHT \
  bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
    --repo "$REPO_FIX" --tag "p14sign-test-only-v0.4.0-light" --expected-version "$VERSION_FIX" \
    --expected-commit "$COMMIT40" --allowed-signers "$ALLOWED" --test-fixture

# Unsigned annotated tag
git -C "$REPO_FIX" tag -a -m "unsigned" "p14sign-test-only-v0.4.0-unsigned"
run_expect_reject SSH_NEG_UNSIGNED \
  bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
    --repo "$REPO_FIX" --tag "p14sign-test-only-v0.4.0-unsigned" --expected-version "$VERSION_FIX" \
    --expected-commit "$COMMIT40" --allowed-signers "$ALLOWED" --test-fixture

BAD_COMMIT="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
run_expect_reject SSH_NEG_WRONG_COMMIT \
  bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
    --repo "$REPO_FIX" --tag "$TAG_NAME" --expected-version "$VERSION_FIX" \
    --expected-commit "$BAD_COMMIT" --allowed-signers "$ALLOWED" --test-fixture

run_expect_reject SSH_NEG_WRONG_VERSION \
  bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
    --repo "$REPO_FIX" --tag "$TAG_NAME" --expected-version "9.9.9" \
    --expected-commit "$COMMIT40" --allowed-signers "$ALLOWED" --test-fixture

run_expect_reject SSH_NEG_LEGACY \
  bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
    --repo "$REPO_FIX" --tag "p14sign-test-only-v0.3.3" --expected-version "0.3.3" \
    --expected-commit "$COMMIT40" --allowed-signers "$ALLOWED" --test-fixture

# Altered tag content: force-replace annotated signed tag with different unsigned annotated
git -C "$REPO_FIX" tag -d "$TAG_NAME" >/dev/null
git -C "$REPO_FIX" tag -a -m "TAMPERED TEST_ONLY" "$TAG_NAME"
run_expect_reject SSH_NEG_ALTERED_TAG \
  bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
    --repo "$REPO_FIX" --tag "$TAG_NAME" --expected-version "$VERSION_FIX" \
    --expected-commit "$COMMIT40" --allowed-signers "$ALLOWED" --test-fixture

# Recreate signed tag for later binding tests
git -C "$REPO_FIX" tag -d "$TAG_NAME" >/dev/null 2>&1 || true
git -C "$REPO_FIX" \
  -c gpg.format=ssh \
  -c user.signingkey="$SSH_DIR/exyonq-test-signing" \
  tag -s -m "TEST_ONLY annotated tag DO_NOT_PUBLISH" "$TAG_NAME"

run_expect_reject SSH_NEG_MISSING_REPO \
  bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
    --repo "$WORK_ROOT/no-such-repo" --tag "$TAG_NAME" --expected-version "$VERSION_FIX" \
    --expected-commit "$COMMIT40" --allowed-signers "$ALLOWED" --test-fixture

# ========== Part B: Cosign blob + unsigned Option A bundle ==========
BUNDLE="$WORK_ROOT/bundle-good"
mkdir -p "$BUNDLE/artifacts"
printf 'exyonq-test payload amd64\n' >"$BUNDLE/artifacts/exyonq-test-linux-amd64.tar.gz"
printf 'exyonq-test payload zip\n' >"$BUNDLE/artifacts/exyonq-test-windows-amd64.zip"

bash "$RELEASE_SCRIPTS/generate-release-manifest.sh" \
  --version "0.0.0-p14sign-fixture" \
  --git-commit "$COMMIT40" \
  --artifacts-dir "$BUNDLE/artifacts" \
  --output "$BUNDLE/release-manifest.json" \
  --test-fixture \
  --force

bash "$RELEASE_SCRIPTS/generate-checksums.sh" \
  --bundle-root "$BUNDLE" \
  --force

run_expect_accept POS_CHECKSUM_INTEGRITY \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$BUNDLE"

run_expect_accept POS_BUNDLE_INTEGRITY \
  bash "$RELEASE_SCRIPTS/verify-release-bundle.sh" --bundle-root "$BUNDLE" --integrity-only

# Cosign ephemeral key
COSIGN_KEY_DIR="$WORK_ROOT/cosign-keys"
mkdir -p "$COSIGN_KEY_DIR" "$WORK_ROOT/pass"
COSIGN_PASS_FILE="$WORK_ROOT/pass/cosign.pass"
umask 077
# Random passphrase; never echo.
p14sign_python3 - <<'PY' >"$COSIGN_PASS_FILE"
import secrets
print(secrets.token_hex(32), end="")
PY
chmod 0600 "$COSIGN_PASS_FILE"
export COSIGN_PASSWORD
COSIGN_PASSWORD="$(cat "$COSIGN_PASS_FILE")"
(
  cd "$COSIGN_KEY_DIR"
  "$COSIGN_BIN" generate-key-pair >/dev/null
)
PRIVATE_KEY_PATHS+=("$COSIGN_KEY_DIR/cosign.key")
# Public key as cosign.pub (and copy name expected by docs)
cp "$COSIGN_KEY_DIR/cosign.pub" "$BUNDLE/exyonq-cosign.pub"

# Sign SHA256SUMS.txt bytes only
(
  cd "$BUNDLE"
  "$COSIGN_BIN" sign-blob \
    --yes \
    --key "$COSIGN_KEY_DIR/cosign.key" \
    --tlog-upload=false \
    --bundle "$BUNDLE/SHA256SUMS.txt.bundle" \
    --output-signature "$BUNDLE/SHA256SUMS.txt.sig" \
    "$BUNDLE/SHA256SUMS.txt" >/dev/null
)

# Confirm SUMS does not include signature/bundle/self
if grep -E 'SHA256SUMS\.txt(\.sig|\.bundle)?$' "$BUNDLE/SHA256SUMS.txt" >/dev/null; then
  die "SHA256SUMS must not include signature/bundle/self"
fi
grep -F "  release-manifest.json" "$BUNDLE/SHA256SUMS.txt" >/dev/null
grep -F "  artifacts/" "$BUNDLE/SHA256SUMS.txt" >/dev/null

run_expect_accept POS_COSIGN_BLOB \
  bash "$RELEASE_SCRIPTS/verify-cosign-blob.sh" \
    --checksums "$BUNDLE/SHA256SUMS.txt" \
    --key "$BUNDLE/exyonq-cosign.pub" \
    --bundle "$BUNDLE/SHA256SUMS.txt.bundle" \
    --signature "$BUNDLE/SHA256SUMS.txt.sig"

run_expect_accept POS_BUNDLE_ARTIFACT_SIG \
  bash "$RELEASE_SCRIPTS/verify-release-bundle.sh" \
    --bundle-root "$BUNDLE" \
    --verify-artifact-signature \
    --cosign-key "$BUNDLE/exyonq-cosign.pub" \
    --cosign-bundle "$BUNDLE/SHA256SUMS.txt.bundle" \
    --cosign-signature "$BUNDLE/SHA256SUMS.txt.sig"

# ========== Part C: Negative matrix N01–N40 ==========
CASES="$WORK_ROOT/cases"
mkdir -p "$CASES"

# N01 artifact modified
clone_bundle "$BUNDLE" "$CASES/N01"
printf 'TAMPER\n' >>"$CASES/N01/artifacts/exyonq-test-linux-amd64.tar.gz"
run_expect_reject N01_ARTIFACT_MODIFIED \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N01"

# N02 artifact missing
clone_bundle "$BUNDLE" "$CASES/N02"
rm -f "$CASES/N02/artifacts/exyonq-test-linux-amd64.tar.gz"
run_expect_reject N02_ARTIFACT_MISSING \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N02"

# N03 artifact additional
clone_bundle "$BUNDLE" "$CASES/N03"
printf 'extra\n' >"$CASES/N03/artifacts/extra-undeclared.tar.gz"
run_expect_reject N03_ARTIFACT_ADDITIONAL \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N03"

# N04 symlink
clone_bundle "$BUNDLE" "$CASES/N04"
rm -f "$CASES/N04/artifacts/exyonq-test-linux-amd64.tar.gz"
ln -s "exyonq-test-windows-amd64.zip" "$CASES/N04/artifacts/exyonq-test-linux-amd64.tar.gz"
run_expect_reject N04_ARTIFACT_SYMLINK \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N04"

# N05 manifest modified (content) — SUMS hash of manifest will mismatch
clone_bundle "$BUNDLE" "$CASES/N05"
p14sign_python3 - <<'PY' "$CASES/N05/release-manifest.json"
import json,sys
p=sys.argv[1]
o=json.load(open(p))
o["created_at"]="1999-01-01T00:00:00Z"
open(p,"w").write(json.dumps(o,indent=2,sort_keys=True)+"\n")
PY
run_expect_reject N05_MANIFEST_MODIFIED \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N05"

# N06 manifest hash mismatch (artifact sha256 field wrong)
clone_bundle "$BUNDLE" "$CASES/N06"
p14sign_python3 - "$CASES/N06" <<'PY'
import hashlib, json, sys
from pathlib import Path
root = Path(sys.argv[1])
man = root / "release-manifest.json"
obj = json.loads(man.read_text())
obj["artifacts"][0]["sha256"] = "0" * 64
man.write_text(json.dumps(obj, indent=2, sort_keys=True) + "\n")

def sha(p: Path) -> str:
    h = hashlib.sha256()
    h.update(p.read_bytes())
    return h.hexdigest()

lines = []
for rel in ["release-manifest.json"] + [a["path"] for a in obj["artifacts"]]:
    lines.append(f"{sha(root / rel)}  {rel}")
lines.sort(key=lambda L: L.split("  ", 1)[1])
(root / "SHA256SUMS.txt").write_text("\n".join(lines) + "\n")
PY
run_expect_reject N06_MANIFEST_HASH_MISMATCH \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N06"

# N07 size mismatch
clone_bundle "$BUNDLE" "$CASES/N07"
p14sign_python3 - "$CASES/N07" <<'PY'
import hashlib, json, sys
from pathlib import Path
root=Path(sys.argv[1])
man=root/"release-manifest.json"
obj=json.loads(man.read_text())
obj["artifacts"][0]["size_bytes"]=1
man.write_text(json.dumps(obj,indent=2,sort_keys=True)+"\n")
def sha(p):
    h=hashlib.sha256(); h.update(p.read_bytes()); return h.hexdigest()
lines=[]
for rel in ["release-manifest.json"]+[a["path"] for a in obj["artifacts"]]:
    lines.append(f"{sha(root/rel)}  {rel}")
lines.sort(key=lambda L: L.split("  ",1)[1])
(root/"SHA256SUMS.txt").write_text("\n".join(lines)+"\n")
PY
run_expect_reject N07_MANIFEST_SIZE_MISMATCH \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N07"

# N08 checksum modified
clone_bundle "$BUNDLE" "$CASES/N08"
p14sign_python3 - <<'PY' "$CASES/N08/SHA256SUMS.txt"
from pathlib import Path
import sys
p=Path(sys.argv[1])
lines=p.read_text().splitlines()
h,path=lines[0].split("  ",1)
# flip first nibble
flip="1" if h[0]=="0" else "0"
lines[0]=flip+h[1:]+"  "+path
p.write_text("\n".join(lines)+"\n")
PY
run_expect_reject N08_CHECKSUM_MODIFIED \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N08"

# N09 malformed
clone_bundle "$BUNDLE" "$CASES/N09"
printf 'not-a-checksum-line\n' >"$CASES/N09/SHA256SUMS.txt"
run_expect_reject N09_CHECKSUM_MALFORMED \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N09"

# N10 duplicate path
clone_bundle "$BUNDLE" "$CASES/N10"
line="$(head -n1 "$CASES/N10/SHA256SUMS.txt")"
printf '%s\n%s\n' "$(cat "$CASES/N10/SHA256SUMS.txt")" "$line" >"$CASES/N10/SHA256SUMS.txt"
run_expect_reject N10_CHECKSUM_DUPLICATE_PATH \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N10"

# N11 uppercase hash
clone_bundle "$BUNDLE" "$CASES/N11"
p14sign_python3 - <<'PY' "$CASES/N11/SHA256SUMS.txt"
from pathlib import Path
import sys
p=Path(sys.argv[1])
out=[]
for line in p.read_text().splitlines():
    h,path=line.split("  ",1)
    out.append(h.upper()+"  "+path)
p.write_text("\n".join(out)+"\n")
PY
run_expect_reject N11_CHECKSUM_UPPERCASE_HASH \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N11"

# N12 absolute path
clone_bundle "$BUNDLE" "$CASES/N12"
p14sign_python3 - <<'PY' "$CASES/N12/SHA256SUMS.txt"
from pathlib import Path
import sys
p=Path(sys.argv[1])
lines=p.read_text().splitlines()
h,path=lines[0].split("  ",1)
lines[0]=f"{h}  /tmp/{path}"
p.write_text("\n".join(lines)+"\n")
PY
run_expect_reject N12_CHECKSUM_ABSOLUTE_PATH \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N12"

# N13 parent traversal
clone_bundle "$BUNDLE" "$CASES/N13"
p14sign_python3 - <<'PY' "$CASES/N13/SHA256SUMS.txt"
from pathlib import Path
import sys
p=Path(sys.argv[1])
lines=p.read_text().splitlines()
h,_=lines[0].split("  ",1)
lines[0]=f"{h}  ../evil"
p.write_text("\n".join(lines)+"\n")
PY
run_expect_reject N13_CHECKSUM_PARENT_TRAVERSAL \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N13"

# N14 includes signature
clone_bundle "$BUNDLE" "$CASES/N14"
h="$(p14sign_sha256_file "$CASES/N14/SHA256SUMS.txt.sig")"
printf '%s  SHA256SUMS.txt.sig\n' "$h" >>"$CASES/N14/SHA256SUMS.txt"
run_expect_reject N14_CHECKSUM_INCLUDES_SIGNATURE \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N14"

# N15 includes itself
clone_bundle "$BUNDLE" "$CASES/N15"
h="$(p14sign_sha256_file "$CASES/N15/SHA256SUMS.txt")"
printf '%s  SHA256SUMS.txt\n' "$h" >>"$CASES/N15/SHA256SUMS.txt"
run_expect_reject N15_CHECKSUM_INCLUDES_ITSELF \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N15"

# N16 signature modified
clone_bundle "$BUNDLE" "$CASES/N16"
p14sign_python3 - <<'PY' "$CASES/N16/SHA256SUMS.txt.sig"
from pathlib import Path
import sys
p=Path(sys.argv[1])
b=bytearray(p.read_bytes())
if not b: raise SystemExit("empty sig")
b[0]^=0xFF
p.write_bytes(bytes(b))
PY
run_expect_reject N16_SIGNATURE_MODIFIED \
  bash "$RELEASE_SCRIPTS/verify-cosign-blob.sh" \
    --checksums "$CASES/N16/SHA256SUMS.txt" \
    --key "$CASES/N16/exyonq-cosign.pub" \
    --signature "$CASES/N16/SHA256SUMS.txt.sig"

# N17 signature missing
clone_bundle "$BUNDLE" "$CASES/N17"
rm -f "$CASES/N17/SHA256SUMS.txt.sig" "$CASES/N17/SHA256SUMS.txt.bundle"
run_expect_reject N17_SIGNATURE_MISSING \
  bash "$RELEASE_SCRIPTS/verify-release-bundle.sh" \
    --bundle-root "$CASES/N17" \
    --verify-artifact-signature \
    --cosign-key "$CASES/N17/exyonq-cosign.pub"

# N18 bundle modified
clone_bundle "$BUNDLE" "$CASES/N18"
p14sign_python3 - <<'PY' "$CASES/N18/SHA256SUMS.txt.bundle"
from pathlib import Path
import sys
p=Path(sys.argv[1])
b=bytearray(p.read_bytes())
b[-1]^=0x01
p.write_bytes(bytes(b))
PY
run_expect_reject N18_BUNDLE_MODIFIED \
  bash "$RELEASE_SCRIPTS/verify-cosign-blob.sh" \
    --checksums "$CASES/N18/SHA256SUMS.txt" \
    --key "$CASES/N18/exyonq-cosign.pub" \
    --bundle "$CASES/N18/SHA256SUMS.txt.bundle"

# N19 bundle missing (signature-only path still needs something — remove both for missing bundle with signature-only verify requiring bundle in release mode)
clone_bundle "$BUNDLE" "$CASES/N19"
rm -f "$CASES/N19/SHA256SUMS.txt.bundle"
run_expect_reject N19_BUNDLE_MISSING \
  bash "$RELEASE_SCRIPTS/verify-cosign-blob.sh" \
    --checksums "$CASES/N19/SHA256SUMS.txt" \
    --key "$CASES/N19/exyonq-cosign.pub" \
    --bundle "$CASES/N19/SHA256SUMS.txt.bundle"

# N20 wrong cosign public key
clone_bundle "$BUNDLE" "$CASES/N20"
WRONG_COS="$WORK_ROOT/cosign-keys-wrong"
mkdir -p "$WRONG_COS"
COSIGN_PASSWORD="$(cat "$COSIGN_PASS_FILE")"
(
  cd "$WRONG_COS"
  "$COSIGN_BIN" generate-key-pair >/dev/null
)
PRIVATE_KEY_PATHS+=("$WRONG_COS/cosign.key")
cp "$WRONG_COS/cosign.pub" "$CASES/N20/exyonq-cosign.pub"
run_expect_reject N20_WRONG_COSIGN_PUBLIC_KEY \
  bash "$RELEASE_SCRIPTS/verify-cosign-blob.sh" \
    --checksums "$CASES/N20/SHA256SUMS.txt" \
    --key "$CASES/N20/exyonq-cosign.pub" \
    --signature "$CASES/N20/SHA256SUMS.txt.sig"

# N21 private key as verify key
run_expect_reject N21_PRIVATE_KEY_PASSED_AS_VERIFY_KEY \
  bash "$RELEASE_SCRIPTS/verify-cosign-blob.sh" \
    --checksums "$BUNDLE/SHA256SUMS.txt" \
    --key "$COSIGN_KEY_DIR/cosign.key" \
    --signature "$BUNDLE/SHA256SUMS.txt.sig"

# N22 version mismatch — production tag/version binding (fixture tag ≠ v${version})
# Manifest mutation below is intentional noise; rejection is from tag/version contract.
clone_bundle "$BUNDLE" "$CASES/N22"
p14sign_python3 - "$CASES/N22" <<'PY'
import json,sys
from pathlib import Path
root=Path(sys.argv[1])
man=root/"release-manifest.json"
o=json.loads(man.read_text())
o["version"]="0.4.0"
o["git_tag"]="v0.4.0"
man.write_text(json.dumps(o,indent=2,sort_keys=True)+"\n")
PY
run_expect_reject N22_VERSION_MISMATCH \
  bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
    --repo "$REPO_FIX" --tag "$TAG_NAME" --expected-version "1.2.3" \
    --expected-commit "$COMMIT40" --allowed-signers "$ALLOWED" --test-fixture

# Explicit manifest version/tag field mismatch (integrity path)
clone_bundle "$BUNDLE" "$CASES/N22b"
p14sign_python3 - "$CASES/N22b" <<'PY'
import json,sys
from pathlib import Path
root=Path(sys.argv[1])
man=root/"release-manifest.json"
o=json.loads(man.read_text())
o["git_tag"]="v9.9.9"
man.write_text(json.dumps(o,indent=2,sort_keys=True)+"\n")
PY
run_expect_reject N22B_MANIFEST_GIT_TAG_VERSION_MISMATCH \
  bash "$RELEASE_SCRIPTS/verify-release-bundle.sh" --bundle-root "$CASES/N22b" --integrity-only

# N23 tag mismatch (production contract without test-fixture)
run_expect_reject N23_TAG_MISMATCH \
  bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
    --repo "$REPO_FIX" --tag "$TAG_NAME" --expected-version "0.4.0" \
    --expected-commit "$COMMIT40" --allowed-signers "$ALLOWED"

# N24 commit mismatch
run_expect_reject N24_COMMIT_MISMATCH \
  bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
    --repo "$REPO_FIX" --tag "$TAG_NAME" --expected-version "0.4.0" \
    --expected-commit "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" \
    --allowed-signers "$ALLOWED" --test-fixture

# N25 legacy
run_expect_reject N25_LEGACY_VERSION_0_3_3 \
  bash "$RELEASE_SCRIPTS/generate-release-manifest.sh" \
    --version "0.3.3" --git-commit "$COMMIT40" \
    --artifacts-dir "$BUNDLE/artifacts" \
    --output "$CASES/N25-out.json" --force

# N26 latest as version
run_expect_reject N26_LATEST_AS_VERSION_IDENTITY \
  bash "$RELEASE_SCRIPTS/generate-release-manifest.sh" \
    --version "latest" --git-commit "$COMMIT40" \
    --artifacts-dir "$BUNDLE/artifacts" \
    --output "$CASES/N26-out.json" --force

# N27 tag-only OCI
run_expect_reject N27_OCI_REFERENCE_TAG_ONLY \
  bash "$RELEASE_SCRIPTS/verify-oci-signature.sh" \
    --image "ghcr.io/exyonq/exyonq:0.4.0" \
    --key "$BUNDLE/exyonq-cosign.pub" \
    --dry-run-preconditions

# N28 latest OCI
run_expect_reject N28_OCI_REFERENCE_LATEST \
  bash "$RELEASE_SCRIPTS/verify-oci-signature.sh" \
    --image "ghcr.io/exyonq/exyonq:latest" \
    --key "$BUNDLE/exyonq-cosign.pub" \
    --dry-run-preconditions

# N29 malformed digest
run_expect_reject N29_OCI_DIGEST_MALFORMED \
  bash "$RELEASE_SCRIPTS/verify-oci-signature.sh" \
    --image "ghcr.io/exyonq/exyonq@sha256:not-hex" \
    --key "$BUNDLE/exyonq-cosign.pub" \
    --dry-run-preconditions

# N30 wrong length digest
run_expect_reject N30_OCI_DIGEST_WRONG_LENGTH \
  bash "$RELEASE_SCRIPTS/verify-oci-signature.sh" \
    --image "ghcr.io/exyonq/exyonq@sha256:abcd" \
    --key "$BUNDLE/exyonq-cosign.pub" \
    --dry-run-preconditions

# N31 cosign missing
run_expect_reject N31_COSIGN_MISSING \
  env PATH="/usr/bin:/bin" COSIGN_BIN="" \
  bash "$RELEASE_SCRIPTS/verify-cosign-blob.sh" \
    --checksums "$BUNDLE/SHA256SUMS.txt" \
    --key "$BUNDLE/exyonq-cosign.pub" \
    --signature "$BUNDLE/SHA256SUMS.txt.sig"

# N32 git repo missing (already covered; keep dedicated)
run_expect_reject N32_GIT_REPO_MISSING \
  bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
    --repo "$WORK_ROOT/missing-git" --tag "$TAG_NAME" --expected-version "0.4.0" \
    --expected-commit "$COMMIT40" --allowed-signers "$ALLOWED" --test-fixture

# N33 allowed signers missing
run_expect_reject N33_ALLOWED_SIGNERS_MISSING \
  bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
    --repo "$REPO_FIX" --tag "$TAG_NAME" --expected-version "0.4.0" \
    --expected-commit "$COMMIT40" --allowed-signers "$WORK_ROOT/no-signers" --test-fixture

# N34 dirty release tree
clone_bundle "$BUNDLE" "$CASES/N34"
printf 'unexpected\n' >"$CASES/N34/evil-extra.txt"
run_expect_reject N34_DIRTY_RELEASE_TREE \
  bash "$RELEASE_SCRIPTS/verify-release-bundle.sh" --bundle-root "$CASES/N34" --integrity-only

# N35 private key inside bundle
clone_bundle "$BUNDLE" "$CASES/N35"
cp "$COSIGN_KEY_DIR/cosign.key" "$CASES/N35/cosign.key"
run_expect_reject N35_PRIVATE_KEY_INSIDE_BUNDLE \
  bash "$RELEASE_SCRIPTS/verify-release-bundle.sh" --bundle-root "$CASES/N35" --integrity-only

# N36 private key inside security/signing (temp tree)
SEC_TMP="$CASES/N36-security-signing"
mkdir -p "$SEC_TMP"
cp "$REPO_ROOT/security/signing/"* "$SEC_TMP/" 2>/dev/null || true
cp "$COSIGN_KEY_DIR/cosign.key" "$SEC_TMP/cosign.key"
run_expect_reject N36_PRIVATE_KEY_INSIDE_SECURITY_SIGNING \
  bash "$RELEASE_SCRIPTS/scan-private-keys.sh" --root "$SEC_TMP"

# Real security/signing must stay clean
run_expect_accept POS_SECURITY_SIGNING_CLEAN \
  bash "$RELEASE_SCRIPTS/scan-private-keys.sh" --root "$REPO_ROOT/security/signing"

# N37 newline in filename (argument guard)
run_expect_reject N37_NEWLINE_IN_FILENAME \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$BUNDLE"$'\n'evil

# N38 duplicate manifest artifact declaration
clone_bundle "$BUNDLE" "$CASES/N38"
p14sign_python3 - "$CASES/N38" <<'PY'
import json,sys
from pathlib import Path
root=Path(sys.argv[1])
man=root/"release-manifest.json"
o=json.loads(man.read_text())
o["artifacts"].append(dict(o["artifacts"][0]))
man.write_text(json.dumps(o,indent=2,sort_keys=True)+"\n")
PY
run_expect_reject N38_DUPLICATE_MANIFEST_ARTIFACT \
  p14sign_python3 "$RELEASE_SCRIPTS/lib/manifest.py" validate --manifest "$CASES/N38/release-manifest.json"

# N39 undeclared artifact
clone_bundle "$BUNDLE" "$CASES/N39"
printf 'x\n' >"$CASES/N39/artifacts/sneaky.tar.gz"
run_expect_reject N39_UNDECLARED_ARTIFACT \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N39"

# N40 declared artifact missing from disk
clone_bundle "$BUNDLE" "$CASES/N40"
rm -f "$CASES/N40/artifacts/exyonq-test-windows-amd64.zip"
run_expect_reject N40_MANIFEST_DECLARED_ARTIFACT_MISSING \
  bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$CASES/N40"

# ========== Part D: OCI digest binding positive dry-run ==========
FAKE_DIG="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
run_expect_accept POS_OCI_DIGEST_PRECONDITIONS \
  bash "$RELEASE_SCRIPTS/verify-oci-signature.sh" \
    --image "ghcr.io/exyonq/exyonq@sha256:${FAKE_DIG}" \
    --key "$BUNDLE/exyonq-cosign.pub" \
    --dry-run-preconditions

# Network fail must not PASS (registry unreachable). Bound wait — never hang the harness.
run_expect_reject OCI_NETWORK_FAIL_CLOSED \
  perl -e 'alarm 15; exec @ARGV' -- \
  bash "$RELEASE_SCRIPTS/verify-oci-signature.sh" \
    --image "127.0.0.1:1/exyonq/exyonq@sha256:${FAKE_DIG}" \
    --key "$BUNDLE/exyonq-cosign.pub"

# Private key as OCI verify key
run_expect_reject OCI_PRIVATE_KEY_AS_PUB \
  bash "$RELEASE_SCRIPTS/verify-oci-signature.sh" \
    --image "ghcr.io/exyonq/exyonq@sha256:${FAKE_DIG}" \
    --key "$COSIGN_KEY_DIR/cosign.key" \
    --dry-run-preconditions

# ========== CI Option A static check ==========
if grep -q 'build-manifest.json' "$REPO_ROOT/.github/workflows/release.yml" \
  && grep -q 'Write SHA256SUMS (basename-relative' "$REPO_ROOT/.github/workflows/release.yml"; then
  die "CI Option A cutover incomplete (legacy checksum job still present)"
fi
if ! grep -q 'P14SIGN_CI_OPTION_A_CUTOVER=PASS' "$REPO_ROOT/.github/workflows/release.yml"; then
  die "CI Option A cutover marker missing in release.yml"
fi
if ! grep -q 'generate-release-manifest.sh' "$REPO_ROOT/.github/workflows/release.yml"; then
  die "CI must generate release-manifest.json"
fi
if ! grep -q 'generate-checksums.sh' "$REPO_ROOT/.github/workflows/release.yml"; then
  die "CI must generate SHA256SUMS via generate-checksums.sh"
fi
# Detect YAML run+uses collision regression
if grep -nE '\}[[:space:]]+- name:' "$REPO_ROOT/.github/workflows/release.yml" >/dev/null; then
  die "release.yml has run/uses step collision (missing newline before next step)"
fi
run_expect_accept POS_CI_OPTION_A_STATIC \
  bash -c 'test -f "$1/.github/workflows/release.yml"' _ "$REPO_ROOT"

# ========== Final private-key scan of repo public surfaces ==========
run_expect_accept POS_SCAN_SECURITY_SIGNING \
  bash "$RELEASE_SCRIPTS/scan-private-keys.sh" --root "$REPO_ROOT/security/signing"

# Count FAIL lines
FAIL_N="$(grep -c '^FAIL ' "$PASS_FILE" || true)"
PASS_N="$(grep -c '^PASS ' "$PASS_FILE" || true)"

# Machine-readable summary
{
  echo "P14SIGN_PHASE3_HARNESS=COMPLETE"
  echo "WORK_ROOT=$WORK_ROOT"
  echo "PASS_COUNT=$PASS_N"
  echo "FAIL_COUNT=$FAIL_N"
  echo "COSIGN_BIN=$COSIGN_BIN"
  "$COSIGN_BIN" version 2>/dev/null | head -n1 || true
} | tee "$WORK_ROOT/summary.txt"

if [[ "$FAIL_N" != "0" ]]; then
  echo "---- FAIL details ----" >&2
  grep '^FAIL ' "$PASS_FILE" >&2 || true
  die "Phase 3 harness had $FAIL_N failure(s)"
fi

echo "P14SIGN_PHASE3_HARNESS=PASS"
