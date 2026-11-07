#!/usr/bin/env bash
# Negative-path regression suite for EXYONQ-SEC-PRIVATE-MATERIAL-ZERO.
# Plants private material in an isolated sandbox and asserts the scanner FAILs.
# Sample payloads are built at runtime (never stored as contiguous literals here).
set -euo pipefail
LC_ALL=C
export LC_ALL
set +x

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCAN="$ROOT/scripts/security/scan-private-material.sh"
GEN="$ROOT/scripts/test-tls/generate-ephemeral-tls.sh"
chmod +x "$SCAN" "$GEN" 2>/dev/null || true

expect_fail() {
  local label="$1"
  shift
  if "$@"; then
    echo "ERROR: expected FAIL for $label" >&2
    exit 1
  fi
  echo "PASS_EXPECT_FAIL $label"
}

expect_pass() {
  local label="$1"
  shift
  "$@" || { echo "ERROR: expected PASS for $label" >&2; exit 1; }
  echo "PASS_EXPECT_PASS $label"
}

# Build PEM/OpenSSH headers without committing contiguous forbidden blocks.
pem_pkcs8() {
  printf '%s\n%s\n%s\n' \
    "-----BEGIN ""PRIVATE KEY-----" \
    "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7FAKE_NOT_A_REAL_KEY" \
    "-----END ""PRIVATE KEY-----"
}

openssh_priv() {
  printf '%s\n%s\n%s\n' \
    "-----BEGIN ""OPENSSH PRIVATE KEY-----" \
    "b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW" \
    "-----END ""OPENSSH PRIVATE KEY-----"
}

SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-pmz-regress-XXXXXX")"
trap 'rm -rf "$SANDBOX"' EXIT

# --- 1. PEM under tests/fixtures ---
mkdir -p "$SANDBOX/tests/fixtures/tls"
pem_pkcs8 >"$SANDBOX/tests/fixtures/tls/key.pem"
expect_fail pem_under_tests_fixtures bash "$SCAN" --tree "$SANDBOX/tests"

# --- 2. OpenSSH under examples ---
mkdir -p "$SANDBOX/examples"
openssh_priv >"$SANDBOX/examples/id_ed25519"
expect_fail openssh_under_examples bash "$SCAN" --tree "$SANDBOX/examples"

# --- 3. Private key renamed as .txt ---
mkdir -p "$SANDBOX/renamed"
pem_pkcs8 >"$SANDBOX/renamed/notes.txt"
expect_fail renamed_txt bash "$SCAN" --tree "$SANDBOX/renamed"

# --- 4. Embedded in shell script ---
mkdir -p "$SANDBOX/scripts"
{
  echo '#!/usr/bin/env bash'
  echo -n "KEY='"
  pem_pkcs8 | tr '\n' ' '
  echo "'"
} >"$SANDBOX/scripts/embed.sh"
expect_fail embedded_shell bash "$SCAN" --tree "$SANDBOX/scripts"

# --- 5. Inside tar.gz ---
mkdir -p "$SANDBOX/arch_src" "$SANDBOX/arch_only"
pem_pkcs8 >"$SANDBOX/arch_src/secret.pem"
tar -czf "$SANDBOX/arch_only/payload.tar.gz" -C "$SANDBOX/arch_src" secret.pem
rm -rf "$SANDBOX/arch_src"
expect_fail tar_gz bash "$SCAN" --archive "$SANDBOX/arch_only/payload.tar.gz"
expect_fail tar_gz_tree bash "$SCAN" --tree "$SANDBOX/arch_only"

# --- 6. Inside OCI-like layer ---
mkdir -p "$SANDBOX/oci/blobs/sha256" "$SANDBOX/oci_layer_src/etc"
pem_pkcs8 >"$SANDBOX/oci_layer_src/etc/key.pem"
tar -cf "$SANDBOX/oci/blobs/sha256/deadbeeflayer" -C "$SANDBOX/oci_layer_src" .
printf '%s\n' '{"schemaVersion":2}' >"$SANDBOX/oci/index.json"
expect_fail oci_layer bash "$SCAN" --oci "$SANDBOX/oci"

# --- 7. Binary strings ---
mkdir -p "$SANDBOX/bin"
python3 - <<PY
from pathlib import Path
hdr = ("-----BEGIN " + "PRIVATE KEY-----").encode()
end = ("-----END " + "PRIVATE KEY-----").encode()
payload = b"HDR\0" + hdr + b"\nMIIE\n" + end + b"\n\0TAIL"
Path("$SANDBOX/bin/blob.bin").write_bytes(payload)
PY
expect_fail binary_strings bash "$SCAN" --tree "$SANDBOX/bin"

# --- 8. GitHub token-shaped value ---
mkdir -p "$SANDBOX/tok"
printf 'token=ghp_%s\n' "$(python3 -c 'print("A"*40)')" >"$SANDBOX/tok/env.txt"
expect_fail github_pat bash "$SCAN" --tree "$SANDBOX/tok"

# --- 9. Docker config with auth ---
mkdir -p "$SANDBOX/docker"
# Long base64-looking auth blob (not a real credential)
python3 - <<PY
from pathlib import Path
auth = "ZXhhbXBsZTp0b2tlbjEyMzQ1Njc4OTA="
Path("$SANDBOX/docker/config.json").write_text(
    '{"auths":{"ghcr.io":{"auth":"%s"}}}\n' % auth
)
PY
expect_fail docker_auth bash "$SCAN" --tree "$SANDBOX/docker"

# --- 10. Absolute path to EXYONQ-SIGNING-A ---
mkdir -p "$SANDBOX/paths"
printf 'KEY_PATH=/Volumes/LexarSecure/%s/cosign.key\n' "EXYONQ-SIGNING-A" >"$SANDBOX/paths/notes.env"
expect_fail signing_volume_path bash "$SCAN" --tree "$SANDBOX/paths"

# --- 11. Publication mode includes untracked, non-ignored files. ---
# The old --git-tree mode only enumerates tracked paths; this regression keeps
# the publication review from silently missing a canary that a push would ship.
PUBREPO="$SANDBOX/publication-repo"
mkdir -p "$PUBREPO"
git -C "$PUBREPO" init -q
git -C "$PUBREPO" config user.name 'ExyonQ Scanner Test'
git -C "$PUBREPO" config user.email 'scanner-test@exyonq.invalid'
printf 'public tree baseline\n' >"$PUBREPO/tracked.txt"
git -C "$PUBREPO" add tracked.txt
git -C "$PUBREPO" commit -q -m 'scanner publication baseline'
pem_pkcs8 >"$PUBREPO/untracked-canary.txt"
expect_pass publication_git_tree_misses_untracked \
  bash "$SCAN" --git-tree --allow-blind-scan --repo "$PUBREPO"
expect_fail git_tree_requires_explicit_risk_marker \
  bash "$SCAN" --git-tree --repo "$PUBREPO"
expect_fail publication_blocks_untracked_canary \
  bash "$SCAN" --publication --repo "$PUBREPO"
rm -f "$PUBREPO/untracked-canary.txt"
expect_pass publication_clean_after_canary_removal \
  bash "$SCAN" --publication --repo "$PUBREPO"

# A secret staged in the index must be caught even when the working tree is clean.
pem_pkcs8 >"$PUBREPO/tracked.txt"
git -C "$PUBREPO" add tracked.txt
printf 'public tree baseline\n' >"$PUBREPO/tracked.txt"
expect_pass publication_git_tree_misses_staged_secret \
  bash "$SCAN" --git-tree --allow-blind-scan --repo "$PUBREPO"
expect_fail publication_blocks_staged_canary \
  bash "$SCAN" --publication --repo "$PUBREPO"
expect_fail git_index_blocks_staged_canary \
  bash "$SCAN" --git-index --repo "$PUBREPO"
git -C "$PUBREPO" reset --hard -q HEAD
expect_pass publication_clean_after_staged_removal \
  bash "$SCAN" --publication --repo "$PUBREPO"
expect_pass git_index_clean_after_staged_removal \
  bash "$SCAN" --git-index --repo "$PUBREPO"

# Ignored paths are local-only and therefore outside the publication set.
printf 'ignored-local/\n' >"$PUBREPO/.gitignore"
mkdir -p "$PUBREPO/ignored-local"
pem_pkcs8 >"$PUBREPO/ignored-local/canary.txt"
expect_pass publication_excludes_ignored_local \
  bash "$SCAN" --publication --repo "$PUBREPO"

# --- 12. Full reachable history, including files deleted later. ---
HISTREPO="$SANDBOX/history-repo"
mkdir -p "$HISTREPO"
git -C "$HISTREPO" init -q
git -C "$HISTREPO" config user.name 'ExyonQ Scanner Test'
git -C "$HISTREPO" config user.email 'scanner-test@exyonq.invalid'
printf 'history baseline\n' >"$HISTREPO/tracked.txt"
git -C "$HISTREPO" add tracked.txt
git -C "$HISTREPO" commit -q -m 'history baseline'
pem_pkcs8 >"$HISTREPO/deleted-canary.txt"
git -C "$HISTREPO" add deleted-canary.txt
git -C "$HISTREPO" commit -q -m 'history canary commit'
rm -f "$HISTREPO/deleted-canary.txt"
git -C "$HISTREPO" add -A
git -C "$HISTREPO" commit -q -m 'delete history canary'
expect_fail history_blocks_deleted_canary \
  bash "$SCAN" --history --repo "$HISTREPO"

HISTCLEAN="$SANDBOX/history-clean"
mkdir -p "$HISTCLEAN"
git -C "$HISTCLEAN" init -q
git -C "$HISTCLEAN" config user.name 'ExyonQ Scanner Test'
git -C "$HISTCLEAN" config user.email 'scanner-test@exyonq.invalid'
printf 'history clean\n' >"$HISTCLEAN/tracked.txt"
git -C "$HISTCLEAN" add tracked.txt
git -C "$HISTCLEAN" commit -q -m 'history clean'
expect_pass history_clean_repository \
  bash "$SCAN" --history --repo "$HISTCLEAN"

# A regex definition is not key material. This mirrors the historical
# p14sign-common.sh false positive without embedding a real PEM block.
begin="-----BEGIN"
printf '%s\n' \
  "grep -qE '^${begin} RSA PRIVATE KEY-----\$' \"\$p\"" \
  >"$HISTCLEAN/pattern-only.sh"
git -C "$HISTCLEAN" add pattern-only.sh
git -C "$HISTCLEAN" commit -q -m 'pattern definition only'
expect_pass history_ignores_regex_pattern_source \
  bash "$SCAN" --history --repo "$HISTCLEAN"

# --- Positive: public cert only ---
mkdir -p "$SANDBOX/public"
printf '%s\n%s\n%s\n' \
  "-----BEGIN ""CERTIFICATE-----" \
  "MIIBkTCB+wIJAKHBjQzVexampleNOTAREALCERTIFICATeDATA0000000000000000" \
  "-----END ""CERTIFICATE-----" >"$SANDBOX/public/cert.pem"
expect_pass public_cert_only bash "$SCAN" --tree "$SANDBOX/public"

# --- Ephemeral generator ---
eval "$(bash "$GEN")"
test -f "$EXYONQ_TLS_KEY"
test -f "$EXYONQ_TLS_CERT"
DIR1="$EXYONQ_TLS_DIR"
expect_fail ephemeral_dir_has_key bash "$SCAN" --tree "$DIR1"
bash "$GEN" --cleanup "$DIR1"
test ! -d "$DIR1"

if git -C "$ROOT" ls-files --error-unmatch benchmarks/scenarios/fixtures/tls/key.pem >/dev/null 2>&1; then
  echo "P14TLSFIX_KEY_STILL_TRACKED_IN_GIT=YES"
  echo "P14V040_FREEZE_REPOINT_REQUIRED=YES"
else
  echo "P14TLSFIX_KEY_STILL_TRACKED_IN_GIT=NO"
fi

echo "PRIVATE_MATERIAL_SCANNER_UNIT_TEST=PASS"
echo "PRIVATE_MATERIAL_SCANNER_ARCHIVE_TEST=PASS"
echo "PRIVATE_MATERIAL_SCANNER_OCI_TEST=PASS"
echo "PRIVATE_MATERIAL_SCANNER_RENAMED_FILE_TEST=PASS"
echo "PRIVATE_MATERIAL_SCANNER_FALSE_NEGATIVE_GATE=PASS"
echo "PRIVATE_MATERIAL_SCANNER_PUBLICATION_GATE=PASS"
echo "PRIVATE_MATERIAL_SCANNER_PUBLICATION_INDEX_GATE=PASS"
echo "PRIVATE_MATERIAL_SCANNER_INDEX_BLOB_GATE=PASS"
echo "PRIVATE_MATERIAL_SCANNER_HISTORY_GATE=PASS"
echo "PRIVATE_MATERIAL_SCANNER_BLIND_MODE_GATE=PASS"
echo "EXYONQ_SEC_PRIVATE_MATERIAL_ZERO_SELFTEST=PASS"
echo "EXYONQ_REGRESSION_TESTS=PASS"
