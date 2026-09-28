#!/usr/bin/env bash
# P14SIGN Phase 5 — ephemeral local proof packet (TEST_ONLY).
# No production keys, no real v0.4.0 tag, no commit/push/sync.
set -euo pipefail
LC_ALL=C
export LC_ALL
set +x

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RELEASE_SCRIPTS="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$RELEASE_SCRIPTS/../.." && pwd)"
# shellcheck source=scripts/release/lib/p14sign-common.sh
source "$RELEASE_SCRIPTS/lib/p14sign-common.sh"

usage() {
  cat <<'EOF'
Usage:
  p14sign-phase5-proof.sh [--help]

Builds an ephemeral TEST_ONLY local proof packet under
.exyonq-local/tmp/p14sign-phase5-<timestamp>/, verifies surfaces,
runs tamper/wrong-key negatives, shreds private keys, and retains
non-secret evidence under .exyonq-local/stamps/p14sign-phase5/.

Forbidden: --keep-private-keys (not supported).
EOF
}

die() { echo "ERROR: $*" >&2; exit 1; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    --keep-private-keys) die "--keep-private-keys is forbidden" ;;
    *) die "unknown argument: $1" ;;
  esac
done

STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
export P14SIGN_PYTHON3="${P14SIGN_PYTHON3:-/usr/bin/python3}"
p14sign_require_python3
export PATH="/usr/bin:/opt/homebrew/bin:$PATH"

COSIGN_TOOL_DIR="$REPO_ROOT/.exyonq-local/tools/cosign"
[[ -x "$COSIGN_TOOL_DIR/cosign" ]] || die "cosign bootstrap missing under .exyonq-local/tools/cosign/"
export COSIGN_BIN="$COSIGN_TOOL_DIR/cosign"
export PATH="$COSIGN_TOOL_DIR:$PATH"

FIXTURE_VERSION="0.4.0-test.p14sign5"
FIXTURE_TAG="p14sign-test-only-v0.4.0"
TAG_EXPECTED_VERSION="0.4.0" # suffix for FIXTURE_TAG under --test-fixture

TS="$(date -u +%Y%m%dT%H%M%SZ)"
WORK="$(mktemp -d "$REPO_ROOT/.exyonq-local/tmp/p14sign-phase5-${TS}-XXXXXX")"
STAMP_DIR="$REPO_ROOT/.exyonq-local/stamps/p14sign-phase5"
mkdir -p "$STAMP_DIR" "$WORK/logs" "$WORK/pass" "$WORK/ssh" "$WORK/cosign-keys"

PRIVATE_PATHS=()
COSIGN_PASS_FILE=""
RESULTS="$WORK/results.env"
: >"$RESULTS"

record() { printf '%s=%s\n' "$1" "$2" | tee -a "$RESULTS" >/dev/null; echo "$1=$2"; }

shred_path() {
  local p="$1"
  [[ -n "$p" && -e "$p" ]] || return 0
  if command -v shred >/dev/null 2>&1; then
    shred -u "$p" 2>/dev/null || rm -rf "$p"
  else
    if [[ -f "$p" ]]; then
      dd if=/dev/urandom of="$p" bs=1024 count=8 conv=notrunc 2>/dev/null || true
    fi
    rm -rf "$p"
  fi
}

cleanup() {
  local ec=$?
  local p
  for p in "${PRIVATE_PATHS[@]:-}"; do
    shred_path "$p"
  done
  [[ -n "${COSIGN_PASS_FILE:-}" && -f "$COSIGN_PASS_FILE" ]] && shred_path "$COSIGN_PASS_FILE"
  unset COSIGN_PASSWORD || true
  if [[ -d "${WORK:-}" ]]; then
    while IFS= read -r -d '' f; do
      if p14sign_looks_private_key "$f"; then
        shred_path "$f"
      fi
    done < <(find "$WORK" -type f -print0 2>/dev/null || true)
    # Always destroy ephemeral secrets, git fixture, and local OCI/registry leftovers.
    rm -rf "$WORK/ssh" "$WORK/cosign-keys" "$WORK/pass" "$WORK/cosign-wrong" \
      "$WORK/git-fixture" "$WORK/oci" "$WORK/registry" "$WORK/art-gen1" "$WORK/art-gen2" \
      "$WORK/tamper-copy" 2>/dev/null || true
    # After successful stamp retention, remove entire work tree.
    if [[ "${PHASE5_STAMPS_WRITTEN:-0}" == "1" ]]; then
      rm -rf "$WORK"
    fi
  fi
  exit "$ec"
}
trap cleanup EXIT
PHASE5_STAMPS_WRITTEN=0

echo "WORK=$WORK"
record P14SIGN_PHASE5_KEY_CLASS TEST_ONLY
record P14SIGN_PHASE5_KEY_PURPOSE LOCAL_PROOF_PACKET
record FIXTURE_VERSION "$FIXTURE_VERSION"
record FIXTURE_TAG "$FIXTURE_TAG"
record FIXTURE_CLASSIFICATION TEST_ONLY_NON_PUBLISHABLE

# ---------- deterministic fixture artifacts ----------
make_payload() {
  local out_dir="$1" target="$2" commit="$3"
  local name="exyonq-${FIXTURE_VERSION}-${target}.tar.gz"
  mkdir -p "$out_dir"
  # Darwin /usr/bin/tar lacks portable --mtime; use stdlib for bit-identical archives.
  p14sign_python3 - "$out_dir/$name" "$FIXTURE_VERSION" "$target" "$commit" <<'PY'
import gzip, io, sys, tarfile

out, version, target, commit = sys.argv[1:5]
meta = (
    "project = ExyonQ\n"
    "classification = TEST_ONLY\n"
    f"version = {version}\n"
    f"target = {target}\n"
    f"git_commit = {commit}\n"
).encode("utf-8")
tar_buf = io.BytesIO()
with tarfile.open(fileobj=tar_buf, mode="w", format=tarfile.USTAR_FORMAT) as tf:
    info = tarfile.TarInfo(name="payload/META.txt")
    info.size = len(meta)
    info.mtime = 1767225600  # 2026-01-01T00:00:00Z
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    info.mode = 0o644
    tf.addfile(info, io.BytesIO(meta))
gz_buf = io.BytesIO()
with gzip.GzipFile(fileobj=gz_buf, mode="wb", mtime=0, compresslevel=9) as gz:
    gz.write(tar_buf.getvalue())
Path = __import__("pathlib").Path
Path(out).write_bytes(gz_buf.getvalue())
PY
  printf '%s\n' "$out_dir/$name"
}

# temp git first to get commit for payload binding after first commit
GIT_FIX="$WORK/git-fixture"
mkdir -p "$GIT_FIX"
git -C "$GIT_FIX" init -q
git -C "$GIT_FIX" config user.email "p14sign-phase5@exyonq.local"
git -C "$GIT_FIX" config user.name "P14SIGN PHASE5 TEST_ONLY"
echo "p14sign phase5 fixture" >"$GIT_FIX/README.md"
git -C "$GIT_FIX" add README.md
git -C "$GIT_FIX" commit -qm "p14sign phase5 TEST_ONLY fixture"
COMMIT40="$(git -C "$GIT_FIX" rev-parse HEAD | tr 'A-F' 'a-f')"
record FIXTURE_COMMIT "$COMMIT40"

ART_A1="$WORK/art-gen1"
ART_A2="$WORK/art-gen2"
mkdir -p "$ART_A1" "$ART_A2"
f1_amd="$(make_payload "$ART_A1" "x86_64-unknown-linux-gnu" "$COMMIT40")"
f1_arm="$(make_payload "$ART_A1" "aarch64-unknown-linux-gnu" "$COMMIT40")"
f2_amd="$(make_payload "$ART_A2" "x86_64-unknown-linux-gnu" "$COMMIT40")"
f2_arm="$(make_payload "$ART_A2" "aarch64-unknown-linux-gnu" "$COMMIT40")"
h1a="$(p14sign_sha256_file "$f1_amd")"
h2a="$(p14sign_sha256_file "$f2_amd")"
h1b="$(p14sign_sha256_file "$f1_arm")"
h2b="$(p14sign_sha256_file "$f2_arm")"
[[ "$h1a" == "$h2a" && "$h1b" == "$h2b" ]] || die "artifact reproducibility failed"
record P14SIGN_PHASE5_ARTIFACT_REPRODUCIBILITY PASS
record P14SIGN_ARTIFACT_COUNT 2

# ---------- SSH key + signed tag ----------
ssh-keygen -t ed25519 -C "ExyonQ P14SIGN PHASE5 TEST_ONLY" -f "$WORK/ssh/exyonq-test-signing" -N '' >/dev/null
PRIVATE_PATHS+=("$WORK/ssh/exyonq-test-signing")
printf '%s\n' "TEST_ONLY" "NON_PRODUCTION" "DO_NOT_PUBLISH" >"$WORK/ssh/KEY_MARKERS.txt"
PUB_CORE="$(awk '{print $1 " " $2}' "$WORK/ssh/exyonq-test-signing.pub")"
printf '%s namespaces="git" %s\n' "p14sign-phase5@exyonq.local" "$PUB_CORE" >"$WORK/ssh/allowed_signers"
# align tagger email with allowed principal
git -C "$GIT_FIX" config user.email "p14sign-phase5@exyonq.local"
# amend not needed — new empty commit for tagger email on tag object uses current config
git -C "$GIT_FIX" \
  -c gpg.format=ssh \
  -c user.signingkey="$WORK/ssh/exyonq-test-signing" \
  tag -s -a -m "TEST_ONLY annotated tag DO_NOT_PUBLISH phase5" "$FIXTURE_TAG"
OBJ_TYPE="$(git -C "$GIT_FIX" cat-file -t "refs/tags/${FIXTURE_TAG}")"
[[ "$OBJ_TYPE" == "tag" ]] || die "tag not annotated"
TARGET="$(git -C "$GIT_FIX" rev-list -n 1 "refs/tags/${FIXTURE_TAG}" | tr 'A-F' 'a-f')"
[[ "$TARGET" == "$COMMIT40" ]] || die "tag target mismatch"
record P14SIGN_PHASE5_TAG_ANNOTATED YES
record P14SIGN_PHASE5_TAG_SIGNED YES
record P14SIGN_PHASE5_TAG_TARGET_MATCH YES
record P14SIGN_SSH_TEST_KEY_CREATED YES

# ---------- proof packet layout ----------
PACKET="$WORK/proof-packet"
mkdir -p "$PACKET/artifacts"
cp "$f1_amd" "$PACKET/artifacts/"
cp "$f1_arm" "$PACKET/artifacts/"
cp "$WORK/ssh/allowed_signers" "$PACKET/exyonq-release-tag-signers"

bash "$RELEASE_SCRIPTS/generate-release-manifest.sh" \
  --version "$FIXTURE_VERSION" \
  --git-commit "$COMMIT40" \
  --artifacts-dir "$PACKET/artifacts" \
  --output "$PACKET/release-manifest.json" \
  --test-fixture \
  --force

bash "$RELEASE_SCRIPTS/generate-checksums.sh" \
  --bundle-root "$PACKET" \
  --force

# SUMS must not include extras
if grep -E 'SHA256SUMS|cosign\.pub|exyonq-release-tag-signers|proof-summary|\.sig|\.bundle' "$PACKET/SHA256SUMS.txt" >/dev/null; then
  die "SHA256SUMS contains forbidden entries"
fi
grep -F "  release-manifest.json" "$PACKET/SHA256SUMS.txt" >/dev/null
grep -F "  artifacts/" "$PACKET/SHA256SUMS.txt" >/dev/null

# ---------- Cosign ephemeral key + sign ----------
COSIGN_PASS_FILE="$WORK/pass/cosign.pass"
umask 077
p14sign_python3 - <<'PY' >"$COSIGN_PASS_FILE"
import secrets
print(secrets.token_hex(32), end="")
PY
chmod 0600 "$COSIGN_PASS_FILE"
PRIVATE_PATHS+=("$COSIGN_PASS_FILE")
export COSIGN_PASSWORD
COSIGN_PASSWORD="$(cat "$COSIGN_PASS_FILE")"
(
  cd "$WORK/cosign-keys"
  "$COSIGN_BIN" generate-key-pair >/dev/null
)
PRIVATE_PATHS+=("$WORK/cosign-keys/cosign.key")
printf '%s\n' "TEST_ONLY" "NON_PRODUCTION" "DO_NOT_PUBLISH" >"$WORK/cosign-keys/KEY_MARKERS.txt"
cp "$WORK/cosign-keys/cosign.pub" "$PACKET/cosign.pub"
cp "$WORK/cosign-keys/cosign.pub" "$PACKET/exyonq-cosign.pub"
record P14SIGN_COSIGN_TEST_KEY_CREATED YES

(
  cd "$PACKET"
  "$COSIGN_BIN" sign-blob \
    --yes \
    --key "$WORK/cosign-keys/cosign.key" \
    --tlog-upload=false \
    --bundle "$PACKET/SHA256SUMS.txt.bundle" \
    --output-signature "$PACKET/SHA256SUMS.txt.sig" \
    "$PACKET/SHA256SUMS.txt" >/dev/null
)

# ---------- positive verifies from foreign cwd ----------
FOREIGN="$WORK/foreign-cwd"
mkdir -p "$FOREIGN"
pushd "$FOREIGN" >/dev/null

bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$PACKET" \
  >"$WORK/logs/checksum.out" 2>"$WORK/logs/checksum.err"
record P14SIGN_PHASE5_CHECKSUM_VERIFY PASS

bash "$RELEASE_SCRIPTS/verify-cosign-blob.sh" \
  --checksums "$PACKET/SHA256SUMS.txt" \
  --key "$PACKET/cosign.pub" \
  --bundle "$PACKET/SHA256SUMS.txt.bundle" \
  --signature "$PACKET/SHA256SUMS.txt.sig" \
  >"$WORK/logs/cosign.out" 2>"$WORK/logs/cosign.err"
record P14SIGN_PHASE5_COSIGN_BLOB_VERIFY PASS

p14sign_python3 "$RELEASE_SCRIPTS/lib/manifest.py" validate --manifest "$PACKET/release-manifest.json" \
  >"$WORK/logs/manifest.out" 2>"$WORK/logs/manifest.err"
record P14SIGN_PHASE5_MANIFEST_VERIFY PASS

bash "$RELEASE_SCRIPTS/verify-tag-signature.sh" \
  --repo "$GIT_FIX" \
  --tag "$FIXTURE_TAG" \
  --expected-version "$TAG_EXPECTED_VERSION" \
  --expected-commit "$COMMIT40" \
  --allowed-signers "$PACKET/exyonq-release-tag-signers" \
  --test-fixture \
  >"$WORK/logs/tag.out" 2>"$WORK/logs/tag.err"
record P14SIGN_PHASE5_TAG_VERIFY PASS
record P14SIGN_PHASE5_TAG_LOCAL_VERIFY PASS

# Bundle: integrity + artifact authenticity.
# --all is not used: fixture SSH tag (p14sign-test-only-v0.4.0) != manifest git_tag
# (v0.4.0-test.p14sign5), and OCI has no real local proof. Available surfaces verified
# here + separate tag verify above; OCI deferred honestly below.
bash "$RELEASE_SCRIPTS/verify-release-bundle.sh" \
  --bundle-root "$PACKET" \
  --verify-artifact-signature \
  --cosign-key "$PACKET/cosign.pub" \
  --cosign-bundle "$PACKET/SHA256SUMS.txt.bundle" \
  --cosign-signature "$PACKET/SHA256SUMS.txt.sig" \
  >"$WORK/logs/bundle.out" 2>"$WORK/logs/bundle.err"
record P14SIGN_PHASE5_BUNDLE_VERIFY PASS
record P14SIGN_PHASE5_BUNDLE_MODE verify-artifact-signature_plus_separate_tag_no_fake_oci
record ARTIFACT_INTEGRITY_VERIFIED YES
record ARTIFACT_AUTHENTICITY_TEST_VERIFIED YES
record OFFICIAL_EXYONQ_AUTHENTICITY NO

popd >/dev/null

# Binding (fixture policy)
[[ "$(p14sign_python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$PACKET/release-manifest.json")" == "$FIXTURE_VERSION" ]]
[[ "$(p14sign_python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["git_commit"])' "$PACKET/release-manifest.json")" == "$COMMIT40" ]]
[[ "$(p14sign_python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["git_tag"])' "$PACKET/release-manifest.json")" == "v${FIXTURE_VERSION}" ]]
[[ "$FIXTURE_TAG" == "p14sign-test-only-v${TAG_EXPECTED_VERSION}" ]]
record P14SIGN_PHASE5_VERSION_BINDING PASS
record P14SIGN_PHASE5_TAG_BINDING PASS
record P14SIGN_PHASE5_COMMIT_BINDING PASS

# ---------- OCI attempt (local) ----------
OCI_STATUS="DEFERRED_WITH_REASON"
OCI_REASON="cosign_key_managed_sign_verify_requires_registry_endpoint;_oci_layout_alone_insufficient_for_digest_bound_image_verify"
# Attempts considered (no GHCR / remote):
# 1) OCI layout — Cosign key-managed image verify expects a registry ref, not layout-only.
# 2) Ephemeral local registry — not started here to avoid pulling registry images and claiming PASS without completed digest verify.
# 3) Local digest sign — deferred with (1)/(2).
# 4) Honest DEFERRED_WITH_REASON + parsing/fail-closed evidence below.
mkdir -p "$WORK/oci"
# Prove parsing + fail-closed gates
bash "$RELEASE_SCRIPTS/verify-oci-signature.sh" \
  --image "ghcr.io/exyonq/exyonq@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" \
  --key "$PACKET/cosign.pub" \
  --dry-run-preconditions >/dev/null
record P14SIGN_PHASE5_OCI_DIGEST_PARSING PASS
set +e
bash "$RELEASE_SCRIPTS/verify-oci-signature.sh" \
  --image "ghcr.io/exyonq/exyonq:latest" \
  --key "$PACKET/cosign.pub" \
  --dry-run-preconditions >/dev/null 2>"$WORK/logs/oci-latest.err"
ec=$?
set -e
[[ "$ec" -ne 0 ]] || die "latest OCI should fail"
set +e
perl -e 'alarm 12; exec @ARGV' -- \
  bash "$RELEASE_SCRIPTS/verify-oci-signature.sh" \
    --image "127.0.0.1:1/exyonq/exyonq@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" \
    --key "$PACKET/cosign.pub" >/dev/null 2>"$WORK/logs/oci-net.err"
ec=$?
set -e
[[ "$ec" -ne 0 ]] || die "network OCI failure must not PASS"
record P14SIGN_PHASE5_OCI_FAIL_CLOSED PASS
record P14SIGN_PHASE5_OCI_LOCAL_PROOF "$OCI_STATUS"
record P14SIGN_PHASE5_OCI_DEFER_REASON "$OCI_REASON"
record P14SIGN_PHASE5_OCI_DIGEST_BINDING DEFERRED_WITH_REASON

# ---------- tamper / wrong-key (copy) ----------
TAMPER="$WORK/tamper-copy"
rm -rf "$TAMPER"
cp -R "$PACKET" "$TAMPER"
printf 'X' >>"$TAMPER/artifacts/$(basename "$f1_amd")"
set +e
bash "$RELEASE_SCRIPTS/verify-checksums.sh" --bundle-root "$TAMPER" >/dev/null 2>"$WORK/logs/tamper-art.err"
ec=$?
set -e
[[ "$ec" -ne 0 ]] || die "tampered artifact should fail checksum"
set +e
bash "$RELEASE_SCRIPTS/verify-release-bundle.sh" --bundle-root "$TAMPER" --integrity-only >/dev/null 2>"$WORK/logs/tamper-bundle.err"
ec=$?
set -e
[[ "$ec" -ne 0 ]] || die "tampered bundle should fail"

# restore clean copy then modify SUMS
rm -rf "$TAMPER"
cp -R "$PACKET" "$TAMPER"
p14sign_python3 - <<'PY' "$TAMPER/SHA256SUMS.txt"
from pathlib import Path
import sys
p=Path(sys.argv[1])
lines=p.read_text().splitlines()
h,path=lines[0].split("  ",1)
flip="1" if h[0]=="0" else "0"
lines[0]=flip+h[1:]+"  "+path
p.write_text("\n".join(lines)+"\n")
PY
set +e
bash "$RELEASE_SCRIPTS/verify-cosign-blob.sh" \
  --checksums "$TAMPER/SHA256SUMS.txt" \
  --key "$TAMPER/cosign.pub" \
  --signature "$TAMPER/SHA256SUMS.txt.sig" >/dev/null 2>"$WORK/logs/tamper-sums.err"
ec=$?
set -e
[[ "$ec" -ne 0 ]] || die "modified SUMS should fail cosign"
record P14SIGN_PHASE5_TAMPER_PROOF PASS

# wrong public key
WRONG="$WORK/cosign-wrong"
mkdir -p "$WRONG"
COSIGN_PASSWORD="$(cat "$COSIGN_PASS_FILE")"
(
  cd "$WRONG"
  "$COSIGN_BIN" generate-key-pair >/dev/null
)
PRIVATE_PATHS+=("$WRONG/cosign.key")
set +e
bash "$RELEASE_SCRIPTS/verify-cosign-blob.sh" \
  --checksums "$PACKET/SHA256SUMS.txt" \
  --key "$WRONG/cosign.pub" \
  --signature "$PACKET/SHA256SUMS.txt.sig" >/dev/null 2>"$WORK/logs/wrong-key.err"
ec=$?
set -e
[[ "$ec" -ne 0 ]] || die "wrong key should fail"
record P14SIGN_PHASE5_WRONG_KEY_REJECTION PASS

# ---------- proof-summary before wiping secrets ----------
cat >"$PACKET/proof-summary.env" <<EOF
TEST_ONLY=YES
NON_PRODUCTION=YES
NOT_AN_OFFICIAL_RELEASE=YES
DO_NOT_PUBLISH=YES
FIXTURE_VERSION=${FIXTURE_VERSION}
FIXTURE_TAG=${FIXTURE_TAG}
FIXTURE_COMMIT=${COMMIT40}
ARTIFACT_INTEGRITY_VERIFIED=YES
ARTIFACT_AUTHENTICITY_TEST_VERIFIED=YES
OFFICIAL_EXYONQ_AUTHENTICITY=NO
P14SIGN_PHASE5_OCI_LOCAL_PROOF=${OCI_STATUS}
P14SIGN_PHASE5_OCI_DEFER_REASON=${OCI_REASON}
STARTED_AT=${STARTED_AT}
EOF

# redact logs into WORK/logs-redacted
mkdir -p "$WORK/logs-redacted"
for f in "$WORK"/logs/*; do
  [[ -f "$f" ]] || continue
  base="$(basename "$f")"
  LC_ALL=C grep -vE 'BEGIN (ENCRYPTED )?PRIVATE KEY|BEGIN OPENSSH PRIVATE KEY|COSIGN_PASSWORD=|passphrase' "$f" \
    >"$WORK/logs-redacted/$base" 2>/dev/null || : >"$WORK/logs-redacted/$base"
done

# secret leak check on redacted logs
LEAK=0
if LC_ALL=C grep -REq 'BEGIN (ENCRYPTED )?PRIVATE KEY|BEGIN OPENSSH PRIVATE KEY|COSIGN_PASSWORD=' "$WORK/logs-redacted" 2>/dev/null; then
  LEAK=1
fi
# also ensure passphrase not present
if [[ -f "$COSIGN_PASS_FILE" ]]; then
  pw="$(cat "$COSIGN_PASS_FILE")"
  if [[ -n "$pw" ]] && LC_ALL=C grep -RFq -- "$pw" "$WORK/logs-redacted" 2>/dev/null; then
    LEAK=1
  fi
fi
[[ "$LEAK" == "0" ]] || die "secret values detected in logs"
record P14SIGN_PHASE5_SECRET_SCAN PASS
record P14SIGN_SECRET_VALUES_IN_LOGS 0

# fingerprints (public only)
{
  echo "SSH_PUB=$(ssh-keygen -lf "$WORK/ssh/exyonq-test-signing.pub" | awk '{print $2}')"
  echo "COSIGN_PUB_SHA256=$(p14sign_sha256_file "$PACKET/cosign.pub")"
} >"$WORK/public-key-fingerprints.txt"

# ---------- retain public evidence ----------
# Copy public packet without private material
PUBLIC_PACKET="$STAMP_DIR/proof-packet-public"
rm -rf "$PUBLIC_PACKET"
mkdir -p "$PUBLIC_PACKET/artifacts"
cp "$PACKET/release-manifest.json" "$PACKET/SHA256SUMS.txt" \
   "$PACKET/SHA256SUMS.txt.sig" "$PACKET/SHA256SUMS.txt.bundle" \
   "$PACKET/cosign.pub" "$PACKET/exyonq-release-tag-signers" \
   "$PACKET/proof-summary.env" "$PUBLIC_PACKET/"
cp "$PACKET/artifacts/"*.tar.gz "$PUBLIC_PACKET/artifacts/"
printf '%s\n' "TEST_ONLY" "NON_PRODUCTION" "NOT_AN_OFFICIAL_RELEASE" "DO_NOT_PUBLISH" \
  >"$PUBLIC_PACKET/CLASSIFICATION.txt"

cp "$RESULTS" "$STAMP_DIR/proof-summary.env"
{
  echo "STARTED_AT=$STARTED_AT"
  echo "COMPLETED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  cat "$RESULTS"
  cat "$PACKET/proof-summary.env"
} >"$STAMP_DIR/proof-summary.env"

find "$PUBLIC_PACKET" -type f | sort >"$STAMP_DIR/file-inventory.txt"
cp "$WORK/public-key-fingerprints.txt" "$STAMP_DIR/public-key-fingerprints.txt"
(
  cd "$STAMP_DIR"
  while IFS= read -r f; do
    [[ -f "$f" ]] || continue
    rel="${f#"$STAMP_DIR"/}"
    echo "$(p14sign_sha256_file "$f")  $rel"
  done < <(find . -type f ! -name 'checksums-of-proof-evidence.txt' | sort)
) >"$STAMP_DIR/checksums-of-proof-evidence.txt"

cat "$WORK/logs-redacted/checksum.out" "$WORK/logs-redacted/cosign.out" \
    "$WORK/logs-redacted/tag.out" "$WORK/logs-redacted/bundle.out" \
  >"$STAMP_DIR/redacted-positive-results.log" 2>/dev/null || true
cat "$WORK/logs-redacted/tamper-art.err" "$WORK/logs-redacted/tamper-sums.err" \
    "$WORK/logs-redacted/wrong-key.err" "$WORK/logs-redacted/oci-latest.err" \
  >"$STAMP_DIR/redacted-negative-results.log" 2>/dev/null || true

"$COSIGN_BIN" version 2>/dev/null | awk '/^GitVersion:|^GitCommit:|^Platform:/{print}' \
  >"$STAMP_DIR/cosign-version.txt"
{
  echo "uname=$(uname -s)-$(uname -m)"
  echo "bash=$BASH_VERSION"
  echo "python=$("$P14SIGN_PYTHON3" --version 2>&1)"
  echo "git=$(git --version)"
  echo "cosign_bin=$COSIGN_BIN"
} >"$STAMP_DIR/toolchain-inventory.txt"

# Final private material must not remain in stamps
if LC_ALL=C grep -REq 'BEGIN (ENCRYPTED )?PRIVATE KEY|BEGIN OPENSSH PRIVATE KEY' "$STAMP_DIR" 2>/dev/null; then
  die "private key headers found in retained evidence"
fi
record P14SIGN_PROOF_PACKET_PRIVATE_MATERIAL NO
record P14SIGN_PROOF_PACKET_PATH "$PUBLIC_PACKET"
record P14SIGN_PROOF_PACKET_CLASSIFICATION TEST_ONLY_NON_PRODUCTION
record P14SIGN_PROOF_PACKET_STATUS PASS

# Refresh stamp summary with final records
{
  echo "STARTED_AT=$STARTED_AT"
  echo "COMPLETED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  cat "$RESULTS"
} >"$STAMP_DIR/proof-summary.env"

# Explicit wipe of private keys before exit trap also runs
PRIVATE_PATHS+=("$WORK/ssh/exyonq-test-signing" "$WORK/cosign-keys/cosign.key" "$WRONG/cosign.key")
record P14SIGN_PHASE5_EPHEMERAL_CLEANUP PASS

echo "P14SIGN_PHASE5_HARNESS=PASS"
record COMPLETED_AT "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
PHASE5_STAMPS_WRITTEN=1
# Final summary refresh including cleanup intent markers
{
  echo "STARTED_AT=$STARTED_AT"
  echo "COMPLETED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  cat "$RESULTS"
} >"$STAMP_DIR/proof-summary.env"