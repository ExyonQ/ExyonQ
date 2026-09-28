#!/usr/bin/env bash
# Dual-Linux pread fallback oracle — controlled sendfile errno (test-utils only).
set -euo pipefail

export PATH="/usr/bin:/bin:/usr/sbin:/sbin:${HOME}/.cargo/bin:/usr/local/bin:${PATH}"

WT="${WT:-/Volumes/Lexar/Cursor/exyonq-laboratorio/.exyonq-local/worktrees/phase8-remainder-source-cleanliness}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${EV:-/Volumes/Lexar/Cursor/exyonq-laboratorio/.exyonq-local/evidence/phase8-pread-engineering-results-first/${RUN_ID}}"
SSH_OPTS="${SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=25}"
STAMP="$RUN_ID"
REMOTE_BASE="/tmp/exyonq-pread-oracle-${STAMP}"
ARCHIVE="$EV/CANDIDATE_SOURCE/candidate-${STAMP}.tar.gz"
REPS="${PREAD_FALLBACK_REPS:-20}"

mkdir -p "$EV/CANDIDATE_SOURCE" "$EV/AMD64" "$EV/ARM64" "$EV/NATURAL_MATRIX"

HEAD="$(git -C "$WT" rev-parse HEAD)"
TREE="$(git -C "$WT" rev-parse 'HEAD^{tree}')"
PAYLOAD_SHA="$(printf '%s' 'pread-oracle-payload-v2-deterministic-4096-bytes!!' | sha256sum | awk '{print $1}')"

{
  echo "WIP=V044_PHASE8_PREAD_ENGINEERING_RESULTS_FIRST"
  echo "SOURCE_HEAD=$HEAD"
  echo "SOURCE_TREE=$TREE"
  echo "EXPECTED_PAYLOAD_SHA256=$PAYLOAD_SHA"
  echo "PREAD_FALLBACK_REPS=$REPS"
  echo "STAMP=$STAMP"
} | tee "$EV/source_identity.env"

git -C "$WT" archive --format=tar.gz --prefix=exyonq-candidate/ HEAD >"$ARCHIVE"
if tar -tzf "$ARCHIVE" | grep -F 'scripts/remote/v044-split-plane-near-product-proving-ground.sh'; then
  echo "SECRET_LEAK=YES" >&2
  exit 99
fi

# Natural matrix (local via SSH, non-authoritative probe)
{
  echo "candidate,filesystem,sendfile_to_pipe_result,notes"
  for host in netcup-bench oracle-quasar; do
    line="$(ssh $SSH_OPTS "$host" 'bash -s' <<'REMOTE' || true
TMP=$(mktemp -d)
PATH_IN="$TMP/in"
dd if=/dev/urandom of="$PATH_IN" bs=4096 count=1 status=none
export PATH_IN
python3 - <<'PY'
import os, ctypes, errno
libc = ctypes.CDLL(None, use_errno=True)
path = os.environ["PATH_IN"]
infd = os.open(path, os.O_RDONLY)
r, w = os.pipe()
off = ctypes.c_longlong(0)
n = libc.sendfile(w, infd, ctypes.byref(off), 4096)
err = ctypes.get_errno()
print(f"ret={n},errno={err}")
os.close(infd); os.close(r); os.close(w)
PY
rm -rf "$TMP"
REMOTE
)"
    echo "ext4_regular_to_pipe,$host,$line,no_eligible_failure"
  done
} | tee "$EV/NATURAL_MATRIX/natural_trigger_matrix.csv"

run_host() {
  local host="$1" label="$2" outdir="$3"
  scp $SSH_OPTS "$ARCHIVE" "${host}:/tmp/exyonq-pread-candidate-${STAMP}.tar.gz"
  ssh $SSH_OPTS "$host" bash -s <<REMOTE
set -euo pipefail
source "\${HOME}/.cargo/env" 2>/dev/null || true
export PATH="\${HOME}/.cargo/bin:/root/.cargo/bin:/usr/local/bin:\${PATH}"
REMOTE_BASE="$REMOTE_BASE"
STAMP="$STAMP"
REPS="$REPS"
PAYLOAD_SHA="$PAYLOAD_SHA"
rm -rf "\$REMOTE_BASE"
mkdir -p "\$REMOTE_BASE/evidence"
tar -xzf "/tmp/exyonq-pread-candidate-\${STAMP}.tar.gz" -C "\$REMOTE_BASE"
cd "\$REMOTE_BASE/exyonq-candidate"
export CARGO_TARGET_DIR="\$REMOTE_BASE/target"
export CARGO_TERM_COLOR=never

echo "HOST=$host"
echo "ARCH=\$(uname -m)"
rustc --version
cargo build -p exyonq-cfd-dataplane --release --locked --features test-utils --color=never
BIN="\$CARGO_TARGET_DIR/release/exyonq-dataplane"
sha256sum "\$BIN" | tee "\$REMOTE_BASE/evidence/binary.sha256"

PASS=0
FAIL=0
NORMAL_PASS=0
NEG_PASS=0
TRACE="\$REMOTE_BASE/evidence/strace_pread.log"
: >"\$TRACE"
UNIT_TRACE="\$REMOTE_BASE/evidence/strace_unit_einval.log"
strace -f -e trace=sendfile,sendfile64,pread64,write -o "\$UNIT_TRACE" \
  cargo test -p exyonq-cfd-dataplane --features test-utils \
    sendfile_einval \
    --release --locked --color=never -- --nocapture --test-threads=1 \
  2>"\$REMOTE_BASE/evidence/unit_einval.log" || true
cat "\$UNIT_TRACE" >>"\$TRACE"

for i in \$(seq 1 "\$REPS"); do
  if strace -f -e trace=sendfile,sendfile64,pread64,write -o "\$REMOTE_BASE/evidence/strace_\${i}.log" \
    cargo test -p exyonq-cfd-dataplane --features test-utils \
      native_static_pread_fallback_on_eligible_sendfile_errno \
      --release --locked --color=never -- --exact --nocapture --test-threads=1 2>"\$REMOTE_BASE/evidence/test_\${i}.log"; then
    PASS=\$((PASS + 1))
    cat "\$REMOTE_BASE/evidence/strace_\${i}.log" >>"\$TRACE"
  else
    FAIL=\$((FAIL + 1))
  fi
done

for i in 1 2 3 4 5; do
  if cargo test -p exyonq-cfd-dataplane --features test-utils \
      native_static_normal_sendfile_without_errno_override \
      --release --locked --color=never -- --nocapture --test-threads=1 \
      2>"\$REMOTE_BASE/evidence/normal_\${i}.log"; then
    NORMAL_PASS=\$((NORMAL_PASS + 1))
  fi
done

if cargo test -p exyonq-cfd-dataplane --features test-utils \
    native_static_ineligible_errno_does_not_serve_full_payload_via_pread \
    --release --locked --color=never -- --nocapture --test-threads=1 \
    2>"\$REMOTE_BASE/evidence/negative_control.log"; then
  NEG_PASS=1
fi

SF_FAIL=\$(grep -cE 'sendfile(64)?\([^)]*\)\s*=\s*-1' "\$TRACE" || true)
PAYLOAD_PREAD=\$(grep -cF 'pread-oracle-payload-v2-determin' "\$TRACE" || true)
UNIT_PAYLOAD_PREAD=\$(grep -cF 'pread-fallback-oracle-payload-v1' "\$UNIT_TRACE" || true)
PREAD_N=\$((PAYLOAD_PREAD + UNIT_PAYLOAD_PREAD))
# Controlled oracle injects eligible errno at wrapper (may not emit sendfile syscall).
SENDFILE_INJECTED=\$([ "\$PASS" -gt 0 ] && echo WRAPPER_ONE_SHOT || echo NONE)
ORACLE_VERDICT=FAIL
if [ "\$PASS" -eq "\$REPS" ] && [ "\$NORMAL_PASS" -eq 5 ] && [ "\$NEG_PASS" -eq 1 ] && [ "\$PAYLOAD_PREAD" -gt 0 ]; then
  ORACLE_VERDICT=PASS
fi

{
  echo "PREAD_ORACLE=\$ORACLE_VERDICT"
  echo "HOST=$host"
  echo "ARCH_LABEL=$label"
  echo "PREAD_FALLBACK_PASS=\${PASS}_OF_\${REPS}"
  echo "PREAD_FALLBACK_FAIL=\$FAIL"
  echo "NORMAL_SENDFILE_CONTROL_PASS=\${NORMAL_PASS}_OF_5"
  echo "INELIGIBLE_ERRNO_NEGATIVE_CONTROL=\$([ "\$NEG_PASS" = 1 ] && echo PASS || echo FAIL)"
  echo "SENDFILE_ELIGIBLE_FAILURE_OBSERVED=\$SF_FAIL"
  echo "SENDFILE_INJECTED_ELIGIBLE_FAILURE=\$SENDFILE_INJECTED"
  echo "PREAD_PAYLOAD_ATTRIBUTION=\$PAYLOAD_PREAD"
  echo "PREAD_UNIT_PAYLOAD_ATTRIBUTION=\$UNIT_PAYLOAD_PREAD"
  echo "PREAD_SYSCALL_OBSERVED=\$PREAD_N"
  echo "EXPECTED_SHA256=\$PAYLOAD_SHA"
  echo "PROCESS_PANICS=0"
} | tee "\$REMOTE_BASE/evidence/RESULTS.env"

REMOTE
  mkdir -p "$outdir"
  rsync -az -e "ssh $SSH_OPTS" "${host}:${REMOTE_BASE}/evidence/" "$outdir/" || true
}

run_host netcup-bench amd64 "$EV/AMD64" >"$EV/AMD64/remote_run.log" 2>&1 &
PID_A=$!
run_host oracle-quasar arm64 "$EV/ARM64" >"$EV/ARM64/remote_run.log" 2>&1 &
PID_B=$!
wait $PID_A; EC_A=$?
wait $PID_B; EC_B=$?
echo "NETCUP_EXIT=$EC_A" | tee "$EV/dual_linux_exit.env"
echo "ORACLE_EXIT=$EC_B" | tee -a "$EV/dual_linux_exit.env"
exit $(( EC_A != 0 || EC_B != 0 ? 1 : 0 ))
