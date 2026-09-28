#!/usr/bin/env bash
# Deploy a rebuilt static-mvp admit package to one Linux evidence host and start
# host-admit in the background. Package must already exist (from
# build-static-mvp-admit-package.sh).
#
# USAGE:
#   bash scripts/gates/deploy-static-mvp-admit.sh \
#     --package-root /Volumes/Lexar/Cursor/temp/<name> \
#     --host netcup-bench|oracle-quasar \
#     --label netcup|oracle \
#     --arch amd64|arm64 \
#     --run-id YYYYMMDDTHHMMSSZ
set -euo pipefail
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:${PATH}"

PACKAGE_ROOT=""
HOST=""
LABEL=""
ARCH=""
RUN_ID=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --package-root) PACKAGE_ROOT="${2:-}"; shift 2 ;;
    --host) HOST="${2:-}"; shift 2 ;;
    --label) LABEL="${2:-}"; shift 2 ;;
    --arch) ARCH="${2:-}"; shift 2 ;;
    --run-id) RUN_ID="${2:-}"; shift 2 ;;
    *) echo "ERROR: unknown arg $1" >&2; exit 2 ;;
  esac
done
[[ -n "$PACKAGE_ROOT" && -n "$HOST" && -n "$LABEL" && -n "$ARCH" && -n "$RUN_ID" ]] \
  || { echo "ERROR: missing required args" >&2; exit 2; }

TGZ="$PACKAGE_ROOT/pkg/candidate.tgz"
MAN="$PACKAGE_ROOT/local-meta/candidate.manifest"
RUNNER="$PACKAGE_ROOT/pkg/host-admit.sh"
[[ -f "$TGZ" && -f "$MAN" && -f "$RUNNER" ]] || { echo "ERROR: incomplete package"; exit 2; }

# Provenance: generated runner must match tracked source hash recorded in manifest.
WANT="$(grep '^HOST_ADMIT_SHA256=' "$MAN" | cut -d= -f2)"
GOT="$(/usr/bin/shasum -a 256 "$RUNNER" | /usr/bin/awk '{print $1}')"
[[ "$WANT" == "$GOT" ]] || { echo "ERROR: host-admit hash mismatch want=$WANT got=$GOT"; exit 2; }
grep -q 'HOST_ADMIT_GENERATED_FROM_TRACKED_SOURCE=PASS' "$PACKAGE_ROOT/local-meta/provenance.env"
grep -q 'TEMP_PKG_CANONICAL_DEPENDENCY=NO' "$PACKAGE_ROOT/local-meta/provenance.env"

REMOTE="/tmp/exyonq-smvp-admit-${RUN_ID}-${LABEL}"
ssh -o BatchMode=yes "$HOST" "rm -rf '$REMOTE' && mkdir -p '$REMOTE/src' '$REMOTE/evidence/logs'"
scp -o BatchMode=yes -q "$TGZ" "$HOST:$REMOTE/candidate.tgz"
scp -o BatchMode=yes -q "$MAN" "$HOST:$REMOTE/candidate.manifest"
scp -o BatchMode=yes -q "$RUNNER" "$HOST:$REMOTE/host-admit.sh"

ssh -o BatchMode=yes "$HOST" "bash -lc '
set -euo pipefail
source ~/.cargo/env 2>/dev/null || true
remote=\"$REMOTE\"
tar -xzf \"\$remote/candidate.tgz\" -C \"\$remote/src\" 2>/dev/null \
  || tar --warning=no-unknown-keyword -xzf \"\$remote/candidate.tgz\" -C \"\$remote/src\"
chmod +x \"\$remote/host-admit.sh\"
mkdir -p \"\$remote/evidence/logs\"
nohup bash \"\$remote/host-admit.sh\" \"$LABEL\" \"$ARCH\" \"\$remote\" \
  >\"\$remote/evidence/logs/nohup.out\" 2>\"\$remote/evidence/logs/nohup.err\" &
echo \$! > \"\$remote/runner.pid\"
sleep 2
if kill -0 \$(cat \"\$remote/runner.pid\") 2>/dev/null; then
  echo STARTED
else
  echo FAILED_START
  cat \"\$remote/evidence/logs/nohup.err\" || true
  exit 1
fi
echo REMOTE=\$remote
echo PID=\$(cat \$remote/runner.pid)
'"

echo "DEPLOYED $LABEL $REMOTE"
echo "REMOTE=$REMOTE"
