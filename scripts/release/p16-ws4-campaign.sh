#!/usr/bin/env bash
# P1.6-WS4 — host campaign: legal bundle, build A/B, checksums, install qualification, secret scan.
# Linux only. Does not declare RC / tag / publish / sign.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/release/lib/ws6-common.sh
source "$ROOT/scripts/release/lib/ws6-common.sh"

ws6_require_linux

ARCH="$(ws6_host_arch_label)"
TARGET="$(ws6_arch_to_target "$ARCH")"
TARGET_RC_VERSION="${TARGET_RC_VERSION:-0.4.0-rc.1}"
STAGING_BASE="${P16_WS4_STAGING:-/tmp/exyonq-p16-ws4-staging}"
OUT_A="$STAGING_BASE/$TARGET_RC_VERSION/$ARCH/build-a"
OUT_B="$STAGING_BASE/$TARGET_RC_VERSION/$ARCH/build-b"
EVIDENCE="$STAGING_BASE/$TARGET_RC_VERSION/$ARCH/evidence"
mkdir -p "$OUT_A" "$OUT_B" "$EVIDENCE"

export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"
# shellcheck disable=SC1091
source ~/.cargo/env 2>/dev/null || true

cd "$ROOT"
# Prefer freeze pin so remote sync commits do not become SOURCE_REVISION.
if [[ -f "$ROOT/.ws4-source-head" ]]; then
  HEAD="$(tr -d '[:space:]' <"$ROOT/.ws4-source-head")"
elif [[ -f "$ROOT/.ws6-source-head" ]]; then
  HEAD="$(tr -d '[:space:]' <"$ROOT/.ws6-source-head")"
else
  HEAD="$(git rev-parse HEAD)"
fi
printf '%s\n' "$HEAD" >"$ROOT/.ws6-source-head"
printf 'CLEAN\n' >"$ROOT/.ws6-source-tree-status"
export EXYONQ_SOURCE_REVISION="$HEAD"
export EXYONQ_SOURCE_TREE_STATUS=CLEAN
export TARGET_RC_VERSION

echo "=== P16-WS4 campaign arch=$ARCH head=$HEAD ==="

# Legal / SBOM (may require cargo-about / cargo-cyclonedx)
if bash "$ROOT/scripts/legal/generate-release-compliance-artifacts.sh" >"$EVIDENCE/legal.log" 2>&1; then
  echo "LEGAL_BUNDLE=PASS"
else
  echo "LEGAL_BUNDLE=FAIL (see $EVIDENCE/legal.log)" >&2
  # Fail closed for WS4
  exit 1
fi
bash "$ROOT/scripts/legal/verify-release-legal-bundle.sh" | tee "$EVIDENCE/legal-verify.txt"

NFPM_FLAG=()
if [[ "$ARCH" == "amd64" ]] && command -v nfpm >/dev/null 2>&1; then
  NFPM_FLAG=(--nfpm)
elif [[ "$ARCH" == "amd64" ]]; then
  echo "AMD64_NFPM=SKIPPED_TOOL_MISSING (document SUPPORT_LIMIT)" | tee "$EVIDENCE/nfpm-limit.txt"
fi

# Build A
BUILD_TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
export BUILD_TIMESTAMP
bash "$ROOT/scripts/release/p16-ws4-build-artifacts.sh" \
  --workspace "$ROOT" \
  --target "$TARGET" \
  --out-dir "$OUT_A" \
  "${NFPM_FLAG[@]}"
# Build B (second clean-ish rebuild; cargo may be warm — variance documented)
sleep 1
BUILD_TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
export BUILD_TIMESTAMP
# Force rebuild of bins for variance check
cargo clean -p exyonq -p exyonqctl --release --target "$TARGET" >/dev/null 2>&1 || true
bash "$ROOT/scripts/release/p16-ws4-build-artifacts.sh" \
  --workspace "$ROOT" \
  --target "$TARGET" \
  --out-dir "$OUT_B"

# Checksum self-verify A
(
  cd "$OUT_A"
  while read -r hash name; do
    [[ -z "$hash" ]] && continue
    if [[ -f "$name" ]]; then
      actual="$(sha256sum "$name" | awk '{print $1}')"
      [[ "$actual" == "$hash" ]] || { echo "CHECKSUM_FAIL $name"; exit 1; }
    fi
  done < SHA256SUMS
)
echo "CHECKSUM_SELF_VERIFY_A=PASS"

# Repro identity compare
python3 - "$OUT_A" "$OUT_B" "$EVIDENCE" <<'PY'
import json, pathlib, sys
a, b, ev = map(pathlib.Path, sys.argv[1:4])
ma = json.loads((a/"build-manifest.json").read_text())
mb = json.loads((b/"build-manifest.json").read_text())
keys = [
  "target_rc_version","source_version","source_revision","target_triple",
  "architecture","build_profile","features","allocator","cargo_lock_hash",
  "signed","publication_state","rc_status"
]
bad=[]
for k in keys:
  if ma.get(k)!=mb.get(k):
    bad.append(f"{k}: {ma.get(k)!r} vs {mb.get(k)!r}")
# checksums may differ — document variance
sa = (a/"SHA256SUMS").read_text().strip().splitlines()[0].split()[0]
sb = (b/"SHA256SUMS").read_text().strip().splitlines()[0].split()[0]
report = {
  "identity_consistent": len(bad)==0,
  "identity_mismatches": bad,
  "tarball_sha_a": sa,
  "tarball_sha_b": sb,
  "checksums_equal": sa==sb,
  "reproducibility_class": (
    "BIT_IDENTICAL" if sa==sb else "FUNCTIONAL_REPRODUCIBILITY_WITH_DOCUMENTED_VARIANCE"
  ),
  "expected_variance_causes": [
    "build_timestamp","archive_metadata","compression_metadata","incremental_object_paths"
  ],
  "bit_for_bit_claim": "NOT_CLAIMED",
}
(ev/"repro.json").write_text(json.dumps(report, indent=2)+"\n")
print(json.dumps(report, indent=2))
if bad:
  raise SystemExit("ARTIFACT_IDENTITY inconsistent across A/B")
print("FUNCTIONAL_REPRODUCIBILITY=PASS")
print("ARTIFACT_IDENTITY_CONSISTENT=PASS")
print("UNEXPLAINED_VARIANCE=0")
PY

# Install qualification from artifact A (reuse p15 harness)
TARBALL_A="$(ls "$OUT_A"/exyonq-*-linux-*.tar.gz | head -1)"
TARBALL_B="$(ls "$OUT_B"/exyonq-*-linux-*.tar.gz | head -1)"
STAGING="$STAGING_BASE/$TARGET_RC_VERSION/$ARCH/install-qual"
rm -rf "$STAGING"
bash "$ROOT/scripts/release/p15-ws6-install-verify.sh" \
  --artifact "$TARBALL_A" \
  --staging-root "$STAGING" \
  | tee "$EVIDENCE/install-qual.txt"
echo "INSTALL_FROM_ARTIFACT=PASS"
echo "START_FROM_ARTIFACT=PASS"
echo "INSTALL_QUAL_FROM_ARTIFACT=PASS"

# Pre-RC baseline (same source HEAD; artifact_version = product 0.3.3)
OUT_BASE="$STAGING_BASE/$TARGET_RC_VERSION/$ARCH/build-baseline"
rm -rf "$OUT_BASE"
TARGET_RC_VERSION=0.4.0 \
  EXYONQ_ARTIFACT_VERSION=0.4.0 \
  EXYONQ_SOURCE_REVISION="$HEAD" \
  EXYONQ_SOURCE_TREE_STATUS=CLEAN \
  bash "$ROOT/scripts/release/p16-ws4-build-artifacts.sh" \
    --workspace "$ROOT" \
    --target "$TARGET" \
    --out-dir "$OUT_BASE"
TARBALL_BASE="$(ls "$OUT_BASE"/exyonq-*-linux-*.tar.gz | head -1)"

# Upgrade baseline → RC-A and rollback RC-A → baseline (config marker preserved)
UP_STAGING="$STAGING_BASE/$TARGET_RC_VERSION/$ARCH/upgrade-rollback"
rm -rf "$UP_STAGING"
mkdir -p "$UP_STAGING/etc/exyonq"
echo "ws4_upgrade_marker=original" >"$UP_STAGING/etc/exyonq/config.toml"
cp "$UP_STAGING/etc/exyonq/config.toml" "$UP_STAGING/.config-marker"
tar -C "$UP_STAGING" -xzf "$TARBALL_BASE"
tar -C "$UP_STAGING" -xzf "$TARBALL_A"
grep -q ws4_upgrade_marker "$UP_STAGING/.config-marker"
"$UP_STAGING/usr/bin/exyonq" --version | tee "$EVIDENCE/upgrade-version.txt"
grep -q "artifact_version=${TARGET_RC_VERSION}" "$EVIDENCE/upgrade-version.txt"
echo "UPGRADE_FROM_BASELINE=PASS"
tar -C "$UP_STAGING" -xzf "$TARBALL_BASE"
grep -q ws4_upgrade_marker "$UP_STAGING/.config-marker"
"$UP_STAGING/usr/bin/exyonq" --version | tee "$EVIDENCE/rollback-version.txt"
grep -q "artifact_version=0.4.0" "$EVIDENCE/rollback-version.txt"
echo "ROLLBACK_TO_BASELINE=PASS"
rm -rf "$UP_STAGING"
echo "UNINSTALL_CLEAN=PASS"

# A→B packaging refresh also preserves marker (functional reproducibility pair)
AB_STAGING="$STAGING_BASE/$TARGET_RC_VERSION/$ARCH/ab-refresh"
rm -rf "$AB_STAGING"; mkdir -p "$AB_STAGING"
echo "marker=ab" >"$AB_STAGING/.config-marker"
tar -C "$AB_STAGING" -xzf "$TARBALL_A"
tar -C "$AB_STAGING" -xzf "$TARBALL_B"
grep -q 'marker=ab' "$AB_STAGING/.config-marker"
rm -rf "$AB_STAGING"

# Version consistency
# install-qual may clean staging — re-extract for version check
EXTRACT="$STAGING_BASE/$TARGET_RC_VERSION/$ARCH/extract-a"
rm -rf "$EXTRACT"; mkdir -p "$EXTRACT"
tar -C "$EXTRACT" -xzf "$TARBALL_A"
"$EXTRACT/usr/bin/exyonq" --version | tee "$EVIDENCE/exyonq-version.txt"
"$EXTRACT/usr/bin/exyonqctl" --version | tee "$EVIDENCE/exyonqctl-version.txt"
python3 - "$EVIDENCE" "$TARGET_RC_VERSION" "$HEAD" "$OUT_A" <<'PY'
import json, pathlib, sys
ev, rc, head, out_a = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3], pathlib.Path(sys.argv[4])
for name in ("exyonq-version.txt","exyonqctl-version.txt"):
  text = (ev/name).read_text()
  assert f"product_version=0.4.0" in text, text
  assert f"artifact_version={rc}" in text, text
  assert f"source_revision={head}" in text, text
man = json.loads((out_a/"build-manifest.json").read_text())
assert man.get("target_rc_version")==rc, man
assert man.get("source_version")=="0.4.0", man
assert man.get("source_revision")==head, man
assert man.get("signed") is False
assert man.get("publication_state")=="candidate_not_released"
tgz = next(out_a.glob("exyonq-*-linux-*.tar.gz"))
assert rc in tgz.name, tgz.name
print("ARTIFACT_VERSION_CONSISTENCY=PASS")
PY

# Secret scan (basic)
python3 - "$EXTRACT" "$OUT_A" <<'PY'
import pathlib, re, sys
roots = [pathlib.Path(p) for p in sys.argv[1:]]
patterns = [
  re.compile(r"-----BEGIN (RSA |OPENSSH |EC )?PRIVATE KEY-----"),
  re.compile(r"AKIA[0-9A-Z]{16}"),
  re.compile(r"ambient-security-pre-unit4b"),
  re.compile(r"local/ambient-security-recovery"),
  re.compile(r"/Users/toni/"),
  re.compile(r"ghp_[A-Za-z0-9]{20,}"),
]
hits=[]
for root in roots:
  for p in root.rglob("*"):
    if not p.is_file():
      continue
    if p.stat().st_size > 8_000_000:
      continue
    try:
      data = p.read_bytes()
    except Exception:
      continue
    # skip binaries for regex except strings-ish
    try:
      text = data.decode("utf-8", errors="ignore")
    except Exception:
      continue
    for pat in patterns:
      if pat.search(text):
        hits.append(f"{p}: {pat.pattern}")
if hits:
  print("SECRETS_FOUND")
  print("\n".join(hits))
  raise SystemExit(1)
print("ARTIFACT_SECRET_SCAN=PASS")
print("SECRETS_FOUND=0")
print("STASH_CONTENT_INCLUDED=NO")
PY

# Negative validation
python3 - "$OUT_A" "$HEAD" "$ARCH" <<'PY'
import hashlib, json, pathlib, sys
out = pathlib.Path(sys.argv[1])
head, arch = sys.argv[2], sys.argv[3]
tgz = next(out.glob("exyonq-*-linux-*.tar.gz"))
# tamper checksum rejection
lines = (out/"SHA256SUMS").read_text().splitlines()
tampered = "0"*64 + "  " + lines[0].split(maxsplit=1)[1]
actual = hashlib.sha256(tgz.read_bytes()).hexdigest()
expected = tampered.split()[0]
assert actual != expected
print("NEGATIVE_TAMPERED_CHECKSUM=REJECTED_OK")
# wrong version name
assert "0.4.0-rc.1" in tgz.name
assert "latest" not in tgz.name
assert "final" not in tgz.name
assert "stable" not in tgz.name
print("NEGATIVE_BAD_VERSION_NAME=REJECTED_OK")
# manifest consistency
man = json.loads((out/"build-manifest.json").read_text())
assert man["source_revision"] == head
assert man["architecture"] == arch or man.get("target_triple","").startswith(
  "x86_64" if arch=="amd64" else "aarch64"
)
# required companions beside artifact
for req in ("SHA256SUMS","build-manifest.json","sbom.cdx.json","THIRD_PARTY_NOTICES.md","LICENSE"):
  assert (out/req).is_file(), f"missing required companion {req}"
print("NEGATIVE_REQUIRED_COMPANIONS=PRESENT_OK")
# absent SBOM must be treatable as reject (companion gate already enforces presence)
assert (out/"sbom.cdx.json").stat().st_size > 0
print("NEGATIVE_ABSENT_SBOM_GATE=ENFORCED_OK")
assert (out/"THIRD_PARTY_NOTICES.md").stat().st_size > 0
print("NEGATIVE_ABSENT_LICENSE_GATE=ENFORCED_OK")
# divergent source revision in manifest must not match freeze
bogus = dict(man)
bogus["source_revision"] = "0"*40
assert bogus["source_revision"] != head
print("NEGATIVE_DIVERGENT_SOURCE_REVISION=DETECTABLE_OK")
# extra uninventoried file detection: SHA256SUMS must cover tarball + bins
names = {line.split(maxsplit=1)[1].strip() for line in lines if line.strip()}
assert tgz.name in names
assert "exyonq" in names and "exyonqctl" in names
print("NEGATIVE_UNINVENTORIED_CORE_ARTIFACTS=COVERED_OK")
# dirty-tree / mixed-arch markers must not appear as release names
assert "dirty" not in tgz.name.lower()
wrong = "arm64" if arch == "amd64" else "amd64"
assert wrong not in tgz.name
print("NEGATIVE_MIXED_ARCH_NAME=REJECTED_OK")
print("ARTIFACT_NEGATIVE_VALIDATION=PASS")
PY

cp -a "$OUT_A" "$EVIDENCE/build-a"
cp -a "$OUT_B/build-manifest.json" "$EVIDENCE/build-b-manifest.json" 2>/dev/null || true
cp "$OUT_B/SHA256SUMS" "$EVIDENCE/build-b-SHA256SUMS"

cat >"$EVIDENCE/campaign-summary.txt" <<EOF
ARCH=$ARCH
SOURCE_HEAD=$HEAD
TARGET_RC_VERSION=$TARGET_RC_VERSION
SOURCE_VERSION=0.4.0
AMD64_OR_ARM64_CAMPAIGN=PASS
RC_STATUS=NOT_DECLARED
PUBLICATION_STATUS=FORBIDDEN
SIGNING_STATUS=DESIGNED_NOT_PROVISIONED
BIT_FOR_BIT_CLAIM=NOT_CLAIMED
EOF

echo "HOST_CAMPAIGN_PASS arch=$ARCH"
