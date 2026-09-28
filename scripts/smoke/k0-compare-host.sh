#!/usr/bin/env bash
# K0.6 controlled compare on a single Linux host (ReWrk 30s, all-servers, live rivals).
set -euo pipefail
REPO="${1:?repo path}"
HOST_LABEL="${2:?host label e.g. netcup-amd64}"
K0_ROOT="${K0_COMPARE_ROOT:-$HOME/exyonq-dev-soak/k0.6-compare}"
RUN_STAMP="${K0_RUN_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
DEST="$REPO/benchmarks/results-dev/k0.6-compare/${RUN_STAMP}-${HOST_LABEL}"
LOG="$DEST/compare.log"
mkdir -p "$DEST"

cd "$REPO"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"

case "$HOST_LABEL" in
  netcup-amd64)
    export BENCH_NODE_PRESET=netcup-rs2000
    export BENCH_PROVIDER=netcup
    export BENCH_SHAPE="RS 2000 G12"
    export BENCH_ARCH=x86_64
    export BENCH_CLAIM_LEVEL=official_candidate
    ;;
  oracle-aarch64)
    export BENCH_NODE_PRESET=oracle-a1-official
    export BENCH_PROVIDER=oracle-cloud
    export BENCH_SHAPE="VM.Standard.A1.Flex"
    export BENCH_ARCH=aarch64
    export BENCH_CLAIM_LEVEL=official_arm
    ;;
  *)
    echo "unknown HOST_LABEL=$HOST_LABEL" >&2
    exit 2
    ;;
esac

export BENCH_ALL_SERVERS=1
export BENCH_EXYONQ_ONLY=0
export BENCH_DISABLE_RIVALS_CACHE=1
export BENCH_DURATION=30s
export BENCH_WARMUP_SEC=20
export BENCH_PERF_MODE=docker
export BENCH_LOAD_MODE=ceiling
export BENCH_EPOLL_STATIC=1
export EXYONQ_EPOLL_STATIC=1
export BENCH_P7_MAX_PARALLEL="${BENCH_P7_MAX_PARALLEL:-4}"

{
  echo "K0.6 compare host=$HOST_LABEL stamp=$RUN_STAMP"
  echo "repo=$REPO dest=$DEST"
  uname -a
  date -u
} | tee "$DEST/host-env.txt"

for proj in docker exyonq-k0h-smoke exyonq-k0-baseline exyonq-protector-diag; do
  docker compose -p "$proj" -f "$REPO/benchmarks/docker/docker-compose.bench.yml" down -v 2>/dev/null || true
done
docker compose -f "$REPO/benchmarks/docker/docker-compose.bench.yml" down -v 2>/dev/null || true

echo "[build exyonq-bench]" | tee -a "$LOG"
cargo build -p exyonq-bench -q 2>&1 | tee -a "$LOG"

COMPARE_RC=0
if ! cargo run -p exyonq-bench -- compare \
  --duration 30s \
  --in-docker \
  --all-servers \
  --no-cache \
  --include-roadmap \
  2>&1 | tee -a "$LOG"; then
  COMPARE_RC=1
fi

LATEST=""
if [[ -d "$REPO/benchmarks/results" ]]; then
  LATEST=$(ls -td "$REPO/benchmarks/results"/*/ 2>/dev/null | head -1 || true)
fi

if [[ -z "$LATEST" || ! -f "${LATEST%/}/run_meta.json" ]]; then
  echo "K0.6-INVALID: no compare results directory" | tee "$DEST/gate.txt"
  exit 2
fi

LATEST="${LATEST%/}"
if [[ "$COMPARE_RC" -ne 0 ]]; then
  cp -a "$LATEST/." "$DEST/" 2>/dev/null || true
  echo "K0.6-INVALID: compare command failed (rc=$COMPARE_RC)" | tee "$DEST/gate.txt"
  exit 2
fi

mv "$LATEST" "$DEST/run"
ln -sfn run "$DEST/latest"

# capture-environment.sh is required for verify-official-* (environment_captured in run_meta).
# compare/patch-run-meta runs before we relocate the run dir, so capture + re-patch here.
bash "$REPO/benchmarks/scenarios/perf/capture-environment.sh" "$DEST/run" 2>&1 | tee -a "$LOG"
bash "$REPO/benchmarks/scenarios/perf/patch-run-meta.sh" "$DEST/run" 2>&1 | tee -a "$LOG"

python3 - "$DEST/run" "$HOST_LABEL" <<'PY'
import json
import sys
from pathlib import Path

run_dir, host = Path(sys.argv[1]), sys.argv[2]
meta_path = run_dir / "run_meta.json"
meta = json.loads(meta_path.read_text()) if meta_path.is_file() else {}
meta.update({
    "k0_6": True,
    "k0_gate": host,
    "publishable": False,
    "internal_compare": True,
    "public_claims_forbidden": True,
})
# K0.6 is methodology validation, not publication — force non-publishable even if verify passes.
meta["official_publishable"] = False
meta_path.write_text(json.dumps(meta, indent=2) + "\n")
PY

for marker in INVALID-OOM.txt INVALID-LOADGEN.txt INVALID-ABORTED.txt; do
  if [[ -f "$DEST/run/$marker" ]]; then
    echo "K0.6-INVALID: found $marker" | tee "$DEST/gate.txt"
    exit 2
  fi
done

META="$DEST/run/run_meta.json"
if python3 -c "import json,sys; m=json.load(open(sys.argv[1])); sys.exit(2 if m.get('loadgen_saturated') else 0)" "$META" 2>/dev/null; then
  :
else
  echo "K0.6-INVALID: loadgen_saturated=true" | tee "$DEST/gate.txt"
  exit 2
fi

VERIFY_RC=0
if [[ "$HOST_LABEL" == "oracle-aarch64" ]]; then
  VERIFY_SCRIPT="$REPO/benchmarks/scenarios/perf/verify-official-arm-run.sh"
else
  VERIFY_SCRIPT="$REPO/benchmarks/scenarios/perf/verify-official-run.sh"
fi
if ! bash "$VERIFY_SCRIPT" "$DEST/run" 2>&1 | tee -a "$LOG"; then
  VERIFY_RC=1
fi

if [[ "$VERIFY_RC" -ne 0 ]]; then
  echo "K0.6-INVALID: verify gate failed ($VERIFY_SCRIPT)" | tee "$DEST/gate.txt"
  exit 2
fi

echo "K0.6-PASS" | tee "$DEST/gate.txt"
echo "K0.6 complete: $DEST"
