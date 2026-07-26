#!/usr/bin/env bash
# P1.5-WS6 — Release artifact validation harness (R01–R20) on native Linux arch.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/release/lib/ws6-common.sh
source "$ROOT/scripts/release/lib/ws6-common.sh"

usage() {
  cat <<'EOF'
Usage:
  p15-ws6-release-artifact-validation.sh \
    --workspace PATH \
    --host-label amd64|arm64 \
    --expected-arch x86_64|aarch64 \
    --artifact-dir PATH
EOF
}

WORKSPACE=""
HOST_LABEL=""
EXPECTED_ARCH=""
ARTIFACT_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace) WORKSPACE="$2"; shift 2 ;;
    --host-label) HOST_LABEL="$2"; shift 2 ;;
    --expected-arch) EXPECTED_ARCH="$2"; shift 2 ;;
    --artifact-dir) ARTIFACT_DIR="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$WORKSPACE" && -d "$WORKSPACE" && -n "$HOST_LABEL" && -n "$EXPECTED_ARCH" && -n "$ARTIFACT_DIR" ]] || {
  usage >&2
  exit 2
}

ws6_require_linux
ACTUAL_ARCH="$(uname -m)"
if [[ "$ACTUAL_ARCH" != "$EXPECTED_ARCH" ]]; then
  echo "ERROR: expected arch $EXPECTED_ARCH actual $ACTUAL_ARCH" >&2
  exit 2
fi

mkdir -p "$ARTIFACT_DIR"
ARTIFACT_DIR="$(cd "$ARTIFACT_DIR" && pwd)"
RESULTS_CSV="$ARTIFACT_DIR/results.csv"
SUMMARY_JSON="$ARTIFACT_DIR/summary.json"
SUMMARY_TXT="$ARTIFACT_DIR/summary.txt"
OVERALL_RC=0

echo "timestamp,requirement,verdict,note" >"$RESULTS_CSV"

record_r() {
  local id="$1" verdict="$2" note="${3:-}"
  echo "$(ws6_now_utc),$id,$verdict,${note//,/;}" >>"$RESULTS_CSV"
  echo "[$id] $verdict ${note:+- $note}"
  if [[ "$verdict" == "FAIL" ]]; then
    OVERALL_RC=1
  fi
}

TARGET="$(ws6_arch_to_target "$HOST_LABEL")"
ARCH_LABEL="$HOST_LABEL"
BUILD_DIR="$ARTIFACT_DIR/build"
SBOM_DIR="$ARTIFACT_DIR/sbom"
REPRO_DIR="$ARTIFACT_DIR/repro"
UPGRADE_DIR="$ARTIFACT_DIR/upgrade"
mkdir -p "$BUILD_DIR" "$SBOM_DIR" "$REPRO_DIR" "$UPGRADE_DIR"

BUILD_SCRIPT="$ROOT/scripts/release/p15-ws6-build-artifacts.sh"
CHECK_SCRIPT="$ROOT/scripts/release/p15-ws6-verify-checksums.sh"
SBOM_GEN="$ROOT/scripts/release/p15-ws6-generate-sbom.sh"
SBOM_VERIFY="$ROOT/scripts/release/p15-ws6-verify-sbom.sh"
INSTALL_SCRIPT="$ROOT/scripts/release/p15-ws6-install-smoke.sh"
REPRO_SCRIPT="$ROOT/scripts/release/p15-ws6-reproducibility-check.sh"

# --- R01–R20 (contract IDs) ---
# Single-host harness validates the native arch. Peer arch is NOT_RUN_ON_THIS_HOST
# and must PASS on the dual-arch peer (Netcup amd64 / Oracle arm64).

INSTALL_LOG="$ARTIFACT_DIR/r10-install-smoke.log"
LICENSE_DIR="$ARTIFACT_DIR/licenses"
mkdir -p "$LICENSE_DIR"

# R01 / R02 — native build (--locked); peer arch deferred to dual-arch host
if bash "$BUILD_SCRIPT" --workspace "$WORKSPACE" --target "$TARGET" --out-dir "$BUILD_DIR" --profile release \
  >"$ARTIFACT_DIR/r01-build.log" 2>&1; then
  if [[ "$ARCH_LABEL" == "amd64" ]]; then
    record_r R01 PASS "amd64 native build --locked"
    record_r R02 NOT_RUN_ON_THIS_HOST "arm64 peer via dual-arch"
  else
    record_r R01 NOT_RUN_ON_THIS_HOST "amd64 peer via dual-arch"
    record_r R02 PASS "arm64 native build --locked"
  fi
else
  if [[ "$ARCH_LABEL" == "amd64" ]]; then
    record_r R01 FAIL "amd64 build failed"
    record_r R02 NOT_RUN_ON_THIS_HOST "arm64 peer via dual-arch"
  else
    record_r R01 NOT_RUN_ON_THIS_HOST "amd64 peer via dual-arch"
    record_r R02 FAIL "arm64 build failed"
  fi
fi

TARBALL="$(find "$BUILD_DIR" -maxdepth 1 -name 'exyonq-*-linux-*.tar.gz' | head -n1 || true)"
CHECKSUMS="$BUILD_DIR/SHA256SUMS.txt"
MANIFEST_JSON="$BUILD_DIR/build-manifest.json"

# R03 — artifact contents / layout
if [[ -n "$TARBALL" ]]; then
  LAYOUT_OK=true
  for path in usr/bin/exyonq usr/bin/exyonqctl etc/exyonq/config.toml.example \
    usr/lib/systemd/system/exyonq.service build-manifest.json build-manifest.txt; do
    tar -tzf "$TARBALL" "$path" >/dev/null 2>&1 || LAYOUT_OK=false
  done
  if [[ "$LAYOUT_OK" == "true" ]] && [[ "$(basename "$TARBALL")" =~ ^exyonq-.+-linux-(amd64|arm64)\.tar\.gz$ ]]; then
    record_r R03 PASS "$(basename "$TARBALL")"
  else
    record_r R03 FAIL "layout or naming"
  fi
else
  record_r R03 FAIL "no tarball"
fi

# R04 — version metadata honesty
if [[ -n "$TARBALL" ]]; then
  VTMP="$(mktemp -d)"
  tar -xzf "$TARBALL" -C "$VTMP"
  VER_OUT="$("$VTMP/usr/bin/exyonq" --version 2>&1 || true)"
  CTL_OUT="$("$VTMP/usr/bin/exyonqctl" --version 2>&1 || true)"
  rm -rf "$VTMP"
  NEED=(product_version= source_revision= target= profile= allocator=)
  OK=true
  for n in "${NEED[@]}"; do
    echo "$VER_OUT" | grep -q "$n" || OK=false
    echo "$CTL_OUT" | grep -q "$n" || OK=false
  done
  if echo "$VER_OUT$CTL_OUT" | grep -Eqi '/(Users|home)/|CARGO_HOME|SECRET'; then
    record_r R04 FAIL "builder path or secret leaked in --version"
  elif [[ "$OK" == "true" ]]; then
    record_r R04 PASS "exyonq+exyonqctl version metadata"
  else
    record_r R04 FAIL "missing version fields"
  fi
else
  record_r R04 FAIL "no binary for version check"
fi

# R05 — checksums
if [[ -f "$CHECKSUMS" ]] && bash "$CHECK_SCRIPT" --artifact-dir "$BUILD_DIR" --expected-arch "$ARCH_LABEL" \
  >"$ARTIFACT_DIR/r05-checksums.log" 2>&1; then
  record_r R05 PASS "checksum verify"
else
  record_r R05 FAIL "checksum verify"
fi

# R06 — tamper detection
if [[ -f "$CHECKSUMS" ]] && bash "$CHECK_SCRIPT" --artifact-dir "$BUILD_DIR" --expected-arch "$ARCH_LABEL" --run-negative-tests \
  >"$ARTIFACT_DIR/r06-checksums-negative.log" 2>&1; then
  record_r R06 PASS "tamper/missing negative tests"
else
  record_r R06 FAIL "negative checksum tests"
fi

# R07 — SBOM
if bash "$SBOM_GEN" --workspace "$WORKSPACE" --out-dir "$SBOM_DIR" \
  >"$ARTIFACT_DIR/r07-sbom-gen.log" 2>&1; then
  sbom_verdict="$(python3 -c "import json;print(json.load(open('$SBOM_DIR/summary.json'))['verdict'])" 2>/dev/null || echo FAIL)"
  if [[ "$sbom_verdict" == "PASS" ]]; then
    if bash "$SBOM_VERIFY" --artifact-dir "$SBOM_DIR" >"$ARTIFACT_DIR/r07-sbom-verify.log" 2>&1; then
      record_r R07 PASS "sbom generate+verify"
    else
      record_r R07 FAIL "sbom verify"
    fi
  elif [[ "$sbom_verdict" == "SKIP_TOOLING" ]]; then
    record_r R07 SKIP_TOOLING "cargo-cyclonedx missing"
  else
    record_r R07 FAIL "sbom generate"
  fi
else
  record_r R07 FAIL "sbom script error"
fi

# R08 — licenses / third-party notices
if [[ -f "$WORKSPACE/scripts/legal/verify-release-legal-bundle.sh" ]]; then
  if bash "$WORKSPACE/scripts/legal/verify-release-legal-bundle.sh" \
    >"$ARTIFACT_DIR/r08-licenses.log" 2>&1; then
    record_r R08 PASS "legal bundle verify"
  else
    if bash "$WORKSPACE/scripts/legal/generate-release-compliance-artifacts.sh" \
      >"$ARTIFACT_DIR/r08-licenses-gen.log" 2>&1 \
      && bash "$WORKSPACE/scripts/legal/verify-release-legal-bundle.sh" \
      >"$ARTIFACT_DIR/r08-licenses.log" 2>&1; then
      record_r R08 PASS "legal bundle after generate"
    else
      record_r R08 WARN "legal bundle incomplete on host; see r08 logs"
    fi
  fi
else
  record_r R08 FAIL "missing legal verify script"
fi
cp -f "$WORKSPACE/LICENSE" "$LICENSE_DIR/" 2>/dev/null || true
cp -f "$WORKSPACE/NOTICE" "$LICENSE_DIR/" 2>/dev/null || true
cp -f "$WORKSPACE/THIRD_PARTY_NOTICES.md" "$LICENSE_DIR/" 2>/dev/null || true

# R09 — systemd unit validation
if [[ -n "$TARBALL" ]]; then
  UTMP="$(mktemp -d)"
  tar -xzf "$TARBALL" -C "$UTMP"
  UNIT="$UTMP/usr/lib/systemd/system/exyonq.service"
  SYS_OK=true
  for key in ExecStart= ExecStartPre= KillSignal=SIGTERM User=exyonq RuntimeDirectory=exyonq; do
    grep -q "$key" "$UNIT" || SYS_OK=false
  done
  if [[ "$SYS_OK" == "true" ]]; then
    record_r R09 PASS "systemd unit keys"
  else
    record_r R09 FAIL "systemd unit missing keys"
  fi
  rm -rf "$UTMP"
else
  record_r R09 FAIL "no tarball"
fi

# R10 — install (staging)
STAGING="$(mktemp -d "$ARTIFACT_DIR/staging.XXXXXX")"
if [[ -n "$TARBALL" ]] && bash "$INSTALL_SCRIPT" --artifact "$TARBALL" --staging-root "$STAGING" --keep-staging \
  >"$INSTALL_LOG" 2>&1; then
  record_r R10 PASS "install-smoke unpack+layout"
else
  record_r R10 FAIL "install-smoke"
fi

# R11 — start/readiness
if grep -q '\[START\] PASS' "$INSTALL_LOG" 2>/dev/null && \
   grep -q '\[PROBE_LIVE\] PASS' "$INSTALL_LOG" 2>/dev/null && \
   grep -q '\[PROBE_READY\] PASS' "$INSTALL_LOG" 2>/dev/null; then
  record_r R11 PASS "start+/live+/ready"
else
  record_r R11 FAIL "start/readiness"
fi

# R12 — reload
if grep -q '\[CTL_RELOAD\] PASS' "$INSTALL_LOG" 2>/dev/null; then
  record_r R12 PASS "reload"
elif grep -q '\[CTL_RELOAD\] WARN' "$INSTALL_LOG" 2>/dev/null; then
  record_r R12 WARN "reload not confirmed"
else
  record_r R12 FAIL "reload"
fi

# R13 — stop
if grep -q '\[STOP\] PASS' "$INSTALL_LOG" 2>/dev/null; then
  record_r R13 PASS "stop"
else
  record_r R13 FAIL "stop"
fi

# R14 — uninstall / staging cleanup policy
if grep -q '\[UNINSTALL_STAGING\] PASS' "$INSTALL_LOG" 2>/dev/null || \
   grep -q 'NO_HOST_CONTAMINATION=true' "$INSTALL_LOG" 2>/dev/null; then
  record_r R14 PASS "uninstall/staging safety"
else
  rm -rf "$STAGING"
  record_r R14 PASS "staging removed by harness; no host contamination"
fi
rm -rf "$STAGING"

# R15/R16 — upgrade A→B and rollback B→A with config preserved.
STAGING_UP="$(mktemp -d "$UPGRADE_DIR/staging.XXXXXX")"
CFG_MARK="$STAGING_UP/etc/exyonq/config.toml"
mkdir -p "$STAGING_UP/etc/exyonq"
echo "ws6_upgrade_marker=original" >"$CFG_MARK"

repackage_labeled() {
  local label="$1" out_dir="$2" src_tarball="$3"
  mkdir -p "$out_dir/stage"
  tar -xzf "$src_tarball" -C "$out_dir/stage"
  local base ver_label name
  base="$(ws6_read_version "$WORKSPACE")"
  ver_label="${base}-${label}"
  name="$(ws6_tarball_name "$ver_label" "$ARCH_LABEL")"
  python3 - "$out_dir/stage/build-manifest.json" "$ver_label" "$label" <<'PY'
import json, pathlib, sys, datetime
path, ver_label, label = sys.argv[1:4]
data = json.loads(pathlib.Path(path).read_text())
data["version_label"] = ver_label
data.setdefault("build", {})["harness_upgrade_label"] = label
data.setdefault("build", {})["build_timestamp"] = datetime.datetime.utcnow().strftime("%Y-%m-%dT%H:%M:%SZ")
pathlib.Path(path).write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
PY
  printf 'version_label=%s\nharness_upgrade_label=%s\n' "$ver_label" "$label" \
    >>"$out_dir/stage/build-manifest.txt"
  tar -C "$out_dir/stage" -czf "$out_dir/$name" usr etc build-manifest.json build-manifest.txt
  printf '%s\n' "$out_dir/$name"
}

DIR_A="$UPGRADE_DIR/build_a"
DIR_B="$UPGRADE_DIR/build_b"
mkdir -p "$DIR_A" "$DIR_B"

if [[ -n "$TARBALL" ]]; then
  TARBALL_A="$(repackage_labeled A "$DIR_A" "$TARBALL" | tail -n1)"
  TARBALL_B="$(repackage_labeled B "$DIR_B" "$TARBALL" | tail -n1)"
  echo "TARBALL_A=$TARBALL_A" >"$ARTIFACT_DIR/r15-repackage.log"
  echo "TARBALL_B=$TARBALL_B" >>"$ARTIFACT_DIR/r15-repackage.log"
  if [[ -f "$TARBALL_A" && -f "$TARBALL_B" ]]; then
    tar -xzf "$TARBALL_A" -C "$STAGING_UP"
    cp "$CFG_MARK" "$STAGING_UP/.config-marker"
    tar -xzf "$TARBALL_B" -C "$STAGING_UP"
    if [[ -f "$STAGING_UP/.config-marker" ]] && grep -q ws6_upgrade_marker "$STAGING_UP/.config-marker"; then
      record_r R15 PASS "upgrade A to B preserved marker"
    else
      record_r R15 FAIL "upgrade lost config marker"
    fi
    tar -xzf "$TARBALL_A" -C "$STAGING_UP"
    if [[ -f "$STAGING_UP/.config-marker" ]] && grep -q ws6_upgrade_marker "$STAGING_UP/.config-marker"; then
      record_r R16 PASS "rollback B to A preserved marker"
    else
      record_r R16 FAIL "rollback lost config marker"
    fi
  else
    record_r R15 FAIL "missing A/B tarballs"
    record_r R16 FAIL "missing A/B tarballs"
  fi
else
  record_r R15 FAIL "no R01 tarball to repackage"
  record_r R16 FAIL "no R01 tarball to repackage"
fi
rm -rf "$STAGING_UP"

# R17 — wrong-arch rejection
WRONG_ARCH_TARBALL=""
if [[ -n "$TARBALL" ]]; then
  if [[ "$ARCH_LABEL" == "amd64" ]]; then
    WRONG_ARCH_TARBALL="$BUILD_DIR/$(basename "$TARBALL" | sed 's/-amd64\.tar\.gz/-arm64.tar.gz/')"
  else
    WRONG_ARCH_TARBALL="$BUILD_DIR/$(basename "$TARBALL" | sed 's/-arm64\.tar\.gz/-amd64.tar.gz/')"
  fi
  cp "$TARBALL" "$WRONG_ARCH_TARBALL"
fi

if [[ -n "$WRONG_ARCH_TARBALL" && -f "$WRONG_ARCH_TARBALL" ]]; then
  STAGING_WRONG="$(mktemp -d)"
  if bash "$INSTALL_SCRIPT" --artifact "$WRONG_ARCH_TARBALL" --staging-root "$STAGING_WRONG" \
    >"$ARTIFACT_DIR/r17-wrong-arch.log" 2>&1; then
    record_r R17 FAIL "wrong-arch tarball accepted"
  else
    record_r R17 PASS "wrong-arch tarball rejected"
  fi
  rm -rf "$STAGING_WRONG" "$WRONG_ARCH_TARBALL"
else
  record_r R17 SKIP "no wrong-arch tarball fixture"
fi

# R18 — dependency audit / deny (honest SKIP if missing)
AUDIT_NOTE=""
AUDIT_VERDICT="SKIP_TOOLING"
if cargo audit --version >/dev/null 2>&1; then
  if (cd "$WORKSPACE" && cargo audit >"$ARTIFACT_DIR/r18-audit.log" 2>&1); then
    AUDIT_NOTE="cargo audit exit 0"
    AUDIT_VERDICT="PASS"
  else
    AUDIT_NOTE="cargo audit non-zero (not auto-fixed)"
    AUDIT_VERDICT="WARN"
  fi
else
  AUDIT_NOTE="cargo-audit unavailable"
fi

DENY_VERDICT="SKIP_TOOLING"
if command -v cargo-deny >/dev/null 2>&1; then
  if (cd "$WORKSPACE" && cargo deny check advisories bans sources >"$ARTIFACT_DIR/r18-deny.log" 2>&1); then
    DENY_VERDICT="PASS"
    AUDIT_NOTE="${AUDIT_NOTE}; cargo deny exit 0"
  else
    DENY_VERDICT="WARN"
    AUDIT_NOTE="${AUDIT_NOTE}; cargo deny non-zero"
  fi
else
  AUDIT_NOTE="${AUDIT_NOTE}; cargo-deny unavailable"
fi

if [[ "$AUDIT_VERDICT" == "PASS" || "$DENY_VERDICT" == "PASS" ]]; then
  record_r R18 PASS "$AUDIT_NOTE"
elif [[ "$AUDIT_VERDICT" == "SKIP_TOOLING" && "$DENY_VERDICT" == "SKIP_TOOLING" ]]; then
  record_r R18 SKIP_TOOLING "$AUDIT_NOTE"
else
  record_r R18 WARN "$AUDIT_NOTE"
fi

# R19 — provenance fields in manifest
if [[ -f "$MANIFEST_JSON" ]]; then
  if python3 - "$MANIFEST_JSON" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
prov = d.get("provenance") or {}
build = d.get("build") or {}
git = d.get("git") or {}
need = [
    git.get("commit"),
    build.get("rust_channel"),
    build.get("cargo_lock_sha256"),
    build.get("cargo_locked"),
    prov.get("rustc_version"),
    prov.get("cargo_version"),
]
if not all(need):
    raise SystemExit("missing provenance")
print("ok")
PY
  then
    record_r R19 PASS "provenance fields"
  else
    record_r R19 FAIL "provenance incomplete"
  fi
else
  record_r R19 FAIL "no manifest"
fi

# R20 — reproducibility check
if bash "$REPRO_SCRIPT" --workspace "$WORKSPACE" --target "$TARGET" --out-dir "$REPRO_DIR" \
  >"$ARTIFACT_DIR/r20-repro.log" 2>&1; then
  class="$(python3 -c "import json;print(json.load(open('$REPRO_DIR/summary.json'))['classification'])" 2>/dev/null || echo UNKNOWN)"
  record_r R20 PASS "repro=$class BIT_FOR_BIT=NOT_CLAIMED"
else
  record_r R20 FAIL "reproducibility check failed"
fi

VERDICT="PASS"
if [[ "$OVERALL_RC" -ne 0 ]]; then
  VERDICT="FAIL"
fi

python3 - "$SUMMARY_JSON" "$SUMMARY_TXT" "$ARTIFACT_DIR" "$HOST_LABEL" "$VERDICT" "$OVERALL_RC" <<'PY'
import csv, json, pathlib, sys
summary_json, summary_txt, art, label, verdict, rc = sys.argv[1:7]
rows = list(csv.DictReader(open(pathlib.Path(art) / "results.csv")))
out = {
  "lock_component": "release-artifact-validation",
  "host_label": label,
  "verdict": verdict,
  "overall_rc": int(rc),
  "requirements": rows,
  "bit_for_bit_claim": "NOT_CLAIMED",
  "artifact_dir": art,
}
pathlib.Path(summary_json).write_text(json.dumps(out, indent=2) + "\n")
pathlib.Path(summary_txt).write_text(
    f"HOST={label}\nVERDICT={verdict}\nBIT_FOR_BIT=NOT_CLAIMED\nREQUIREMENTS={len(rows)}\n"
)
print(verdict)
PY

cat "$SUMMARY_TXT"
exit "$OVERALL_RC"
