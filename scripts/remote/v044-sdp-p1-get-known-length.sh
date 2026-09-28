#!/usr/bin/env bash
# V044 SDP-P1 Netcup P4 value gate (real product binary, EXYONQ_SDP_P1=0 vs 1).
# PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN — integration gate only.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RUN_ID="${RUN_ID:-$(date +%Y%m%d-%H%M%S)}"
EVIDENCE="${ROOT}/.exyonq-local/evidence/sdp-p1/${RUN_ID}"
mkdir -p "${EVIDENCE}"/{meta,correctness,bench,audits}
HOST="${SDP_HOST:-netcup-bench}"
REPS="${REPS:-5}"
DURATION="${DURATION:-30s}"

cat >"${EVIDENCE}/meta/authority.txt" <<EOF
WIP=V044_SPECIALIZED_PROXY_DATAPLANE_SDP_P1_GET_KNOWN_LENGTH
HOST=${HOST}
REPS=${REPS}
DURATION=${DURATION}
PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN
EOF

echo "Evidence dir: ${EVIDENCE}"
echo "Build release binary on ${HOST}, run correctness, then interleaved P4 reps."
echo "Set EXYONQ_SDP_P1=0 for A0 and EXYONQ_SDP_P1=1 EXYONQ_SDP_SHARDS=4 for A1."
echo "Manual/orchestrated execution — see docs/performance/v044-sdp-p1-get-known-length.md"
echo "${EVIDENCE}" >"${EVIDENCE}/meta/path.txt"
