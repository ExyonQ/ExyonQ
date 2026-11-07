#!/usr/bin/env bash
# Single local entrypoint: publication scan -> native release build -> bundle
# scan -> SSH ED25519 signature -> checksum/signature verification.
#
# This produces a LOCAL CANDIDATE, never an official GitHub release. Official
# release still requires an authorized tag, CI-built dual-arch assets and the
# Cosign ceremony. The private key is consumed only by ssh-keygen.
set -euo pipefail
LC_ALL=C
export LC_ALL

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/release/lib/p14sign-common.sh
source "$ROOT/scripts/release/lib/p14sign-common.sh"

WORKSPACE="$ROOT"
VERSION=""
TARGET=""
OUT_DIR=""
SIGNING_KEY=""
ALLOWED_SIGNERS="$ROOT/security/signing/exyonq-release-artifact-signers"
EXPECTED_FP=""
PRINCIPAL="exyonq-release"
CLASSIFICATION="LOCAL_CANDIDATE_NOT_FOR_RELEASE"

usage() {
  cat <<'EOF'
Usage:
  build-signed-release-bundle.sh \
    --signing-key PATH \
    [--workspace PATH] \
    [--version VERSION] \
    [--target TARGET] \
    [--out-dir DIR] \
    [--allowed-signers PATH] \
    [--expected-fingerprint SHA256:...] \
    [--principal exyonq-release]

Builds the native release tarball, creates the P14SIGN Option-A layout, signs
SHA256SUMS.txt with the owner-held ED25519 key, and verifies the signature,
manifest and checksums. The output is always classified as a local candidate.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace) p14sign_require_arg --workspace "${2:-}"; WORKSPACE="$2"; shift 2 ;;
    --version) p14sign_require_arg --version "${2:-}"; VERSION="$2"; shift 2 ;;
    --target) p14sign_require_arg --target "${2:-}"; TARGET="$2"; shift 2 ;;
    --out-dir) p14sign_require_arg --out-dir "${2:-}"; OUT_DIR="$2"; shift 2 ;;
    --signing-key) p14sign_require_arg --signing-key "${2:-}"; SIGNING_KEY="$2"; shift 2 ;;
    --allowed-signers) p14sign_require_arg --allowed-signers "${2:-}"; ALLOWED_SIGNERS="$2"; shift 2 ;;
    --expected-fingerprint) p14sign_require_arg --expected-fingerprint "${2:-}"; EXPECTED_FP="$2"; shift 2 ;;
    --principal) p14sign_require_arg --principal "${2:-}"; PRINCIPAL="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) p14sign_die "unknown argument: $1" ;;
  esac
done

p14sign_require_arg --signing-key "$SIGNING_KEY"
[[ -d "$WORKSPACE" ]] || p14sign_die "workspace not found: $WORKSPACE"
WORKSPACE="$(cd "$WORKSPACE" && pwd)"
[[ -f "$WORKSPACE/Cargo.toml" ]] || p14sign_die "not an ExyonQ workspace: $WORKSPACE"

if [[ -z "$VERSION" ]]; then
  VERSION="$(awk -F= '/^[[:space:]]*version[[:space:]]*=/{gsub(/[ "\t]/,"",$2); print $2; exit}' "$WORKSPACE/Cargo.toml")"
fi
p14sign_require_arg --version "$VERSION"
p14sign_assert_version_policy "$VERSION" 0

if [[ -z "$TARGET" ]]; then
  TARGET="$(rustc -vV | sed -n 's/^host: //p')"
fi
p14sign_require_arg --target "$TARGET"
case "$TARGET" in
  aarch64-apple-darwin) OS_LABEL="macos"; ARCH_LABEL="arm64" ;;
  x86_64-apple-darwin) OS_LABEL="macos"; ARCH_LABEL="amd64" ;;
  aarch64-unknown-linux-gnu) OS_LABEL="linux"; ARCH_LABEL="arm64" ;;
  x86_64-unknown-linux-gnu) OS_LABEL="linux"; ARCH_LABEL="amd64" ;;
  *) p14sign_die "unsupported release target: $TARGET" ;;
esac

COMMIT="$(git -C "$WORKSPACE" rev-parse HEAD)"
p14sign_is_commit40 "$COMMIT" || p14sign_die "invalid source commit"
if [[ -n "$(git -C "$WORKSPACE" status --porcelain)" ]]; then
  SOURCE_TREE_STATUS="dirty"
  DIRTY_BOOL="true"
else
  SOURCE_TREE_STATUS="clean"
  DIRTY_BOOL="false"
fi

if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$ROOT/.exyonq-local/signing/release-$(date -u +%Y%m%dT%H%M%SZ)"
fi
if [[ -e "$OUT_DIR" && -n "$(ls -A "$OUT_DIR" 2>/dev/null || true)" ]]; then
  p14sign_die "out-dir already exists and is not empty: $OUT_DIR"
fi
mkdir -p "$OUT_DIR/logs"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
BUNDLE="$OUT_DIR/bundle"
STAGE="$OUT_DIR/stage"
mkdir -p "$BUNDLE/artifacts" "$STAGE"

if [[ -z "$EXPECTED_FP" || "$EXPECTED_FP" == "auto" ]]; then
  EXPECTED_FP="$(sed -n 's/^SSH_FINGERPRINT[[:space:]]*=[[:space:]]*//p' \
    "$ROOT/security/signing/fingerprints.txt" | head -n1)"
fi

echo "CLASSIFICATION=$CLASSIFICATION"
echo "VERSION=$VERSION"
echo "SOURCE_COMMIT=$COMMIT"
echo "SOURCE_TREE_STATUS=$SOURCE_TREE_STATUS"
echo "TARGET=$TARGET"
echo "OUT_DIR=$OUT_DIR"

# The publication gate must pass before any release artifact is built.
bash "$ROOT/scripts/security/private-material-release-gates.sh" preflight \
  >"$OUT_DIR/logs/preflight.log" 2>&1
echo "PRIVATE_MATERIAL_PUBLICATION_GATE=PASS"

(
  cd "$WORKSPACE"
  export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"
  # shellcheck disable=SC1091
  source ~/.cargo/env 2>/dev/null || true
  export EXYONQ_SOURCE_REVISION="$COMMIT"
  export EXYONQ_OFFICIAL_RELEASE=0
  export EXYONQ_ARTIFACT_VERSION="$VERSION"
  cargo build --release --locked -p exyonq -p exyonqctl -p exyonq-compat-cli \
    --target "$TARGET"
) >"$OUT_DIR/logs/build.log" 2>&1

BIN_ROOT="$WORKSPACE/target/$TARGET/release"
for bin in exyonq exyonqctl exyonq-compat; do
  [[ -x "$BIN_ROOT/$bin" ]] || p14sign_die "missing built binary: $BIN_ROOT/$bin"
  install -m 0755 "$BIN_ROOT/$bin" "$STAGE/$bin"
done
for f in LICENSE NOTICE THIRD_PARTY_NOTICES.md sbom.cdx.json; do
  [[ -f "$WORKSPACE/$f" ]] || p14sign_die "missing release companion: $f"
  install -m 0644 "$WORKSPACE/$f" "$STAGE/$f"
done

cat >"$STAGE/BUILD-MANIFEST.txt" <<EOF
CLASSIFICATION=$CLASSIFICATION
PRODUCT=ExyonQ
VERSION=$VERSION
SOURCE_COMMIT=$COMMIT
SOURCE_TREE_STATUS=$SOURCE_TREE_STATUS
TARGET=$TARGET
PROFILE=release
CARGO_LOCKED=true
BUILT_AT=$(date -u +%Y%m%dT%H%M%SZ)
OFFICIAL_RELEASE=NO
EOF

ARTIFACT_NAME="exyonq-${OS_LABEL}-${ARCH_LABEL}.tar.gz"
ARTIFACT="$BUNDLE/artifacts/$ARTIFACT_NAME"
tar -czf "$ARTIFACT" -C "$STAGE" .
ARTIFACT_SHA="$(p14sign_sha256_file "$ARTIFACT")"
echo "ARTIFACT=$ARTIFACT_NAME"
echo "ARTIFACT_SHA256=$ARTIFACT_SHA"

bash "$ROOT/scripts/release/generate-release-manifest.sh" \
  --version "$VERSION" \
  --git-commit "$COMMIT" \
  --artifacts-dir "$BUNDLE/artifacts" \
  --output "$BUNDLE/release-manifest.json" \
  --force >"$OUT_DIR/logs/manifest.log" 2>&1

p14sign_python3 - "$BUNDLE/release-manifest.json" "$CLASSIFICATION" "$SOURCE_TREE_STATUS" "$TARGET" <<'PY'
import json, sys
from pathlib import Path
path = Path(sys.argv[1])
obj = json.loads(path.read_text(encoding="utf-8"))
obj["classification"] = sys.argv[2]
obj["source_tree_status"] = sys.argv[3]
obj["target"] = sys.argv[4]
obj["git_tag_binding"] = "UNVERIFIED_LOCAL_CANDIDATE"
obj["artifact_signing"] = "SSH_ED25519_V2_OVER_SHA256SUMS"
path.write_text(json.dumps(obj, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

cat >"$BUNDLE/CANDIDATE-NOT-FOR-RELEASE.txt" <<EOF
CLASSIFICATION=$CLASSIFICATION
OFFICIAL_RELEASE=NO
VERSION=$VERSION
SOURCE_COMMIT=$COMMIT
SOURCE_TREE_STATUS=$SOURCE_TREE_STATUS
TARGET=$TARGET
REASON=Official GitHub release requires an authorized v${VERSION} tag, CI-built dual-arch artifacts and the Cosign ceremony. This bundle is a local host candidate only.
EOF

bash "$ROOT/scripts/release/generate-checksums.sh" \
  --bundle-root "$BUNDLE" \
  --force >"$OUT_DIR/logs/checksums.log" 2>&1

bash "$ROOT/scripts/release/sign-release-bundle.sh" \
  --bundle-root "$BUNDLE" \
  --signing-key "$SIGNING_KEY" \
  --allowed-signers "$ALLOWED_SIGNERS" \
  --expected-fingerprint "$EXPECTED_FP" \
  --principal "$PRINCIPAL" \
  --force >"$OUT_DIR/logs/sign.log" 2>&1

bash "$ROOT/scripts/release/verify-release-bundle.sh" \
  --bundle-root "$BUNDLE" \
  --verify-artifact-signature-ssh \
  --ssh-allowed-signers "$ALLOWED_SIGNERS" \
  --ssh-principal "$PRINCIPAL" \
  --ssh-signature "$BUNDLE/SHA256SUMS.txt.sig" \
  >"$OUT_DIR/logs/bundle-verify.log" 2>&1

(
  cd "$BUNDLE"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c SHA256SUMS.txt
  else
    shasum -a 256 -c SHA256SUMS.txt
  fi
) >"$OUT_DIR/logs/checksum-verify.log" 2>&1

bash "$ROOT/scripts/security/private-material-release-gates.sh" final "$BUNDLE" \
  >"$OUT_DIR/logs/bundle-private-material.log" 2>&1
bash "$ROOT/scripts/security/private-material-release-gates.sh" assets "$BUNDLE/artifacts" \
  >"$OUT_DIR/logs/assets-private-material.log" 2>&1

FP_LINE="$(ssh-keygen -lf "$SIGNING_KEY" | awk '{print $2}')"
cat >"$OUT_DIR/signing-result.txt" <<EOF
CLASSIFICATION=$CLASSIFICATION
VERSION=$VERSION
SOURCE_COMMIT=$COMMIT
SOURCE_TREE_STATUS=$SOURCE_TREE_STATUS
TARGET=$TARGET
BUNDLE=$BUNDLE
ARTIFACT=$ARTIFACT_NAME
ARTIFACT_SHA256=$ARTIFACT_SHA
SIGNING_KEY_ID=EXYONQ_RELEASE_TAG_SSH_V2
SIGNING_KEY_FINGERPRINT=$FP_LINE
SIGNED_OBJECT=SHA256SUMS.txt
SIGNATURE_NAMESPACE=file
SIGNATURE_VERIFY=PASS
CHECKSUMS_VERIFY=PASS
PRIVATE_MATERIAL_PUBLICATION_GATE=PASS
PRIVATE_MATERIAL_BUNDLE_GATE=PASS
OFFICIAL_RELEASE=NO
EOF

echo "SIGNING_KEY_FINGERPRINT=$FP_LINE"
echo "SIGNED_OBJECT=SHA256SUMS.txt"
echo "SIGNATURE_VERIFY=PASS"
echo "CHECKSUMS_VERIFY=PASS"
echo "PRIVATE_MATERIAL_BUNDLE_GATE=PASS"
echo "BUNDLE=$BUNDLE"
echo "SIGNING_RESULT=$OUT_DIR/signing-result.txt"
echo "VERDICT=PASS"
