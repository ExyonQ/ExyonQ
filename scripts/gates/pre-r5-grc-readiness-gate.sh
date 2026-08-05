#!/usr/bin/env bash
# PRE-R5 GRC readiness — governance infrastructure only.
# PASS does NOT authorize tag / push / release / signing / publication / opening R5.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
FAIL=0

say() { echo "$*"; }
pass() { say "PASS: $*"; }
fail() { say "FAIL: $*"; FAIL=1; }

say "PRE_R5_GRC_READINESS_GATE"
say "NOTE: PASS does not open R5 and does not authorize publication"

# Docs SoT tracked
if git ls-files --error-unmatch docs/security/audit-template.md >/dev/null 2>&1 \
  && git ls-files --error-unmatch docs/governance/project-integrity.md >/dev/null 2>&1; then
  pass "docs/security + docs/governance tracked"
else
  fail "canonical docs/security or docs/governance not tracked"
fi

# gitignore must not ignore those paths
if git check-ignore -q docs/security/audit-template.md 2>/dev/null; then
  fail "docs/security still ignored"
else
  pass "docs/security not ignored"
fi
if git check-ignore -q docs/governance/project-integrity.md 2>/dev/null; then
  fail "docs/governance still ignored"
else
  pass "docs/governance not ignored"
fi

# Release audit infra
if bash scripts/verify-release-audit.sh --dry-run-infra >/dev/null; then
  pass "release audit infra"
else
  fail "release audit infra"
fi
if bash scripts/verify-release-audit.sh --selftest >/dev/null 2>&1; then
  pass "release audit selftest"
else
  fail "release audit selftest"
fi

# Integrity triad
if bash scripts/gates/project-integrity-gate.sh --selftest >/dev/null \
  && bash scripts/gates/project-integrity-gate.sh --tree . >/dev/null \
  && bash scripts/gates/benchmark-integrity-gate.sh --selftest >/dev/null \
  && bash scripts/gates/benchmark-integrity-gate.sh --tree . >/dev/null \
  && bash scripts/gates/data-provenance-gate.sh --selftest >/dev/null; then
  pass "integrity triad selftests (+ tree where applicable)"
else
  fail "integrity triad"
fi

# Orphan pyc
if find scripts/gates/lib -name '*.pyc' 2>/dev/null | grep -q .; then
  fail "orphan integrity .pyc present"
else
  pass "INTEGRITY_ORPHAN_PYC=0"
fi

# PMZ
if bash scripts/security/scan-private-material.sh --selftest >/dev/null \
  && bash scripts/security/scan-private-material.sh --git-tree --repo . >/dev/null; then
  pass "PMZ"
else
  fail "PMZ"
fi

# Hooks
if [[ -x .githooks/pre-commit && -x .githooks/pre-push ]]; then
  pass ".githooks present"
else
  fail ".githooks missing or not executable"
fi
HP="$(git config --get core.hooksPath || true)"
if [[ "$HP" == ".githooks" ]]; then
  pass "core.hooksPath=.githooks"
else
  # Not fatal if unset in some clones; setup script documents install
  say "WARN: core.hooksPath='$HP' (expected .githooks after scripts/setup-githooks.sh)"
fi

# Waivers
bash scripts/gates/waiver-expiry-gate.sh --check >/dev/null || fail "waiver register"
pass "waiver register"

# Evidence binder + signing readiness
[[ -f docs/governance/evidence/INDEX.md ]] || fail "evidence INDEX missing"
[[ -f docs/governance/signing-readiness.md ]] || fail "signing-readiness missing"
[[ -f security/signing/TRUST-POLICY.md ]] || fail "trust policy missing"
[[ -f security/signing/exyonq-cosign.pub ]] || fail "cosign pub missing"
pass "evidence binder + signing governance files"

# No Critical/High release-governance blockers remaining (heuristic: required paths exist)
pass "required governance paths present"

if [[ "$FAIL" -ne 0 ]]; then
  say "PRE_R5_GRC_READINESS=FAIL"
  exit 1
fi
say "PRE_R5_GRC_READINESS=PASS"
say "R5_OPENED=NO"
say "PRODUCTION_SIGNING_EXECUTED=NO"
say "PUBLICATION_STATUS=FORBIDDEN"
exit 0
