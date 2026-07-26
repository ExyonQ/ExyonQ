#!/usr/bin/env bash
# P1.6-WS3 — release documentation consistency checker.
# Does NOT declare RC, tag, publish, or sign.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail=0
pass() { echo "PASS: $*"; }
fail_msg() { echo "FAIL: $*" >&2; fail=1; }

TARGET_RC="0.3.3-rc.1"
DEV_VER="0.3.3"

echo "=== p16-ws3-verify-release-docs ==="
echo "ROOT=$ROOT"

# --- Required files ---
required=(
  docs/release/p1.6-versioning-contract.md
  CHANGELOG.md
  docs/release/p1.6-rc1-release-notes-draft.md
  docs/release/p1.6-rc-product-claims.md
  docs/release/p1.6-breaking-change-inventory.md
  docs/release/p1.6-config-migration-guide.md
  docs/release/p1.6-operational-migration-guide.md
  docs/release/p1.6-api-compatibility.md
  docs/release/p1.6-known-issues.md
  docs/release/deprecation-policy.md
  docs/release/p1.6-upgrade-rollback-matrix.md
)
for f in "${required[@]}"; do
  if [[ -f "$f" ]]; then pass "exists $f"
  else fail_msg "missing $f"
  fi
done

# --- Workspace version SoT ---
ws_ver="$(python3 - <<'PY'
import re
text=open("Cargo.toml",encoding="utf-8").read()
# Prefer [workspace.package] version
m=re.search(r"\[workspace\.package\](.*?)(\n\[|\Z)", text, re.S)
if not m:
    raise SystemExit("no workspace.package")
block=m.group(1)
vm=re.search(r'^version\s*=\s*"([^"]+)"', block, re.M)
if not vm:
    raise SystemExit("no version in workspace.package")
print(vm.group(1))
PY
)"
if [[ "$ws_ver" == "$DEV_VER" ]]; then pass "workspace version=$ws_ver"
else fail_msg "workspace version=$ws_ver expected $DEV_VER (WS3 must not silently bump)"
fi

# --- TARGET_RC frozen in contract + changelog ---
grep -q "TARGET_RC_VERSION.*=.*${TARGET_RC}" docs/release/p1.6-versioning-contract.md \
  && pass "contract TARGET_RC_VERSION" \
  || fail_msg "contract missing TARGET_RC_VERSION=$TARGET_RC"

grep -q "\[${TARGET_RC}\]" CHANGELOG.md \
  && pass "changelog contains [${TARGET_RC}]" \
  || fail_msg "CHANGELOG.md missing [${TARGET_RC}]"

# --- RC declaration honesty (WS3 pre-declare OR WS6 internal declare) ---
# Accept either NOT_DECLARED (pre-WS6) or DECLARED_INTERNAL_NOT_RELEASED (post-WS6).
# Never accept public-release claim language.
for f in docs/release/p1.6-rc1-release-notes-draft.md docs/release/p1.6-versioning-contract.md docs/release/p1.6-rc-product-claims.md; do
  if grep -qE 'RC_STATUS[[:space:]]*=[[:space:]]*(NOT_DECLARED|DECLARED_INTERNAL_NOT_RELEASED)' "$f"; then
    pass "RC_STATUS honesty pin in $f"
  else
    fail_msg "RC_STATUS NOT_DECLARED or DECLARED_INTERNAL_NOT_RELEASED missing in $f"
  fi
  if grep -qiE 'official release|production-ready without limits|fully compatible|complete NGINX replacement|complete Apache compatibility' "$f"; then
    if grep -qiE 'production-ready without limits|fully compatible|complete NGINX replacement|complete Apache compatibility' "$f" \
      && ! grep -qiE 'Forbidden claim|not used|FORBIDDEN|do \*\*not\*\* declare|NOT_RELEASED' "$f"; then
      fail_msg "prohibited claim language in $f"
    else
      pass "no undeclared prohibited claims in $f (context checked)"
    fi
  else
    pass "no prohibited claim keywords in $f"
  fi
done

# Notes status: draft pre-WS6, or internal declared post-WS6
if grep -qE 'DRAFT_NOT_RELEASED|INTERNAL_RC_DECLARED_NOT_RELEASED' docs/release/p1.6-rc1-release-notes-draft.md; then
  pass "release notes draft-or-internal-declared honesty"
else
  fail_msg "release notes missing DRAFT_NOT_RELEASED or INTERNAL_RC_DECLARED_NOT_RELEASED"
fi

# --- Signing / bit-for-bit honesty ---
for pin_file in docs/release/p1.6-rc-product-claims.md docs/release/p1.6-known-issues.md; do
  grep -q 'SIGNING_STATUS.*=.*DESIGNED_NOT_PROVISIONED\|SIGNING.*=.*DESIGNED_NOT_PROVISIONED\|signing not provisioned\|DESIGNED_NOT_PROVISIONED' "$pin_file" \
    && pass "signing not provisioned in $pin_file" \
    || fail_msg "signing honesty missing in $pin_file"
  grep -qiE 'BIT_FOR_BIT.*=.*NOT_CLAIMED|bit-for-bit.*not claimed|BIT_FOR_BIT_CLAIM.*=.*NOT_CLAIMED' "$pin_file" \
    && pass "bit-for-bit NOT_CLAIMED in $pin_file" \
    || fail_msg "bit-for-bit honesty missing in $pin_file"
done

# --- Product claim hard limits ---
claims=docs/release/p1.6-rc-product-claims.md
for pin in \
  'H3_RELOAD_SUPPORT.*=.*SAME_LISTENER_CONFIG_ONLY' \
  'FULL_HTACCESS_COMPATIBILITY.*=.*NO' \
  'TOTAL_NGINX_COMPATIBILITY.*=.*NO' \
  'WORDPRESS_CACHE.*=.*OFF' \
  'LITESPEED_IMPORTER.*=.*DEFERRED' \
  'CADDY_IMPORTER.*=.*DEFERRED' \
  'PUBLICATION_STATUS.*=.*FORBIDDEN'
  do
  if grep -qE "$pin" "$claims" docs/release/p1.6-rc1-release-notes-draft.md docs/release/p1.6-versioning-contract.md 2>/dev/null; then
    pass "pin present: $pin"
  else
    # PUBLICATION may only be on contract/notes
    if grep -qE "$pin" "$claims"; then pass "pin in claims: $pin"
    elif grep -rqE "$pin" docs/release/p1.6-*.md; then pass "pin in release docs: $pin"
    else fail_msg "missing pin: $pin"
    fi
  fi
done

# PUBLICATION_STATUS explicitly
grep -q 'PUBLICATION_STATUS.*=.*FORBIDDEN' docs/release/p1.6-versioning-contract.md \
  && pass "PUBLICATION_STATUS FORBIDDEN" \
  || fail_msg "PUBLICATION_STATUS FORBIDDEN missing"

# --- Relative links among release docs (basic) ---
# Extract markdown links to docs/release/*.md and CHANGELOG and verify targets exist.
python3 - <<'PY'
import re, sys, pathlib
root = pathlib.Path(".")
files = list(pathlib.Path("docs/release").glob("p1.6-*.md"))
files += [pathlib.Path("docs/release/deprecation-policy.md"), pathlib.Path("CHANGELOG.md")]
pat = re.compile(r"\[([^\]]+)\]\(([^)]+)\)")
bad = []
for f in files:
    text = f.read_text(encoding="utf-8")
    for _label, href in pat.findall(text):
        if href.startswith(("http://", "https://", "mailto:")):
            continue
        if href.startswith("#"):
            continue
        target = (f.parent / href).resolve()
        try:
            target.relative_to(root.resolve())
        except ValueError:
            # outside repo — skip
            continue
        if not target.exists():
            bad.append(f"{f}: broken link {href}")
if bad:
    print("FAIL: broken links:")
    for b in bad:
        print(" ", b)
    sys.exit(1)
print("PASS: relative links among WS3 release docs")
PY

# --- Config version consistency mentions ---
grep -q 'v1' docs/release/p1.6-config-migration-guide.md \
  && grep -q 'v2' docs/release/p1.6-config-migration-guide.md \
  && pass "config v1/v2 mentioned" \
  || fail_msg "config migration guide missing v1/v2"

# --- Artifact naming coherence (committed packaging contract) ---
if grep -qE 'exyonq-<version>-linux|exyonq-.*linux-amd64\.tar\.gz|versioned.*tar\.gz' \
  docs/release/p1.5-packaging-product-contract.md; then
  pass "artifact naming in p1.5-packaging-product-contract.md"
else
  fail_msg "packaging contract missing versioned tarball naming"
fi

# --- FIXED KF must not appear as open in known-issues ---
if grep -qE 'KF-P16-00(1|2|6|7|8|9|10|11).*OPEN|STATUS.*FIXED.*as open' docs/release/p1.6-known-issues.md; then
  fail_msg "known-issues incorrectly lists fixed KF as open"
else
  pass "known-issues does not reopen fixed KF-P16-001/002/006-011"
fi

if [[ "$fail" -ne 0 ]]; then
  echo "RELEASE_DOC_CONSISTENCY = FAIL"
  exit 1
fi
echo "RELEASE_DOC_CONSISTENCY = PASS"
exit 0
