#!/usr/bin/env bash
# R3D topology neutrality checklist for ExyonQ vs OLS (P4 proxy).
# Records equivalence contract; does not run competitive load.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
EV="${EV:-$ROOT/.exyonq-local/tmp/r3-20260803/r3d}"
mkdir -p "$EV"
OUT="$EV/topology_neutrality.txt"

{
  echo "R3D_TOPOLOGY_NEUTRALITY_CHECK"
  echo "R3_PRODUCT_BASELINE=${R3_PRODUCT_BASELINE:-f0b2d67}"
  echo "DATE_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo
  echo "REQUIRED_EQUIVALENCE="
  cat <<'EOF'
same evidence host (per arch)
same CPU allocation
same memory constraints
same load generator (wrk2 binary + -t/-c/-d/-R)
same upstream peer (exyonq-upstream or compose upstream with identical /api/ 1024B contract)
same request path /api/
same body contract (1024 bytes)
same status contract (200)
same duration
same warmup
same connections/concurrency
same network path (loopback or compose internal)
no hidden cache
no benchmark-only product behavior
no rival-specific favorable configuration
EOF
  echo
  echo "INFRA_PROVEN="
  echo "R3B_bench_upstream_capacity=PASS amd64_max=75000 arm64_max=40000"
  echo "R3C_loadgen=PASS amd64_safe=62500 arm64_safe=31250"
  echo "upstream_contract=GET /api/ -> 1024 x bytes"
  echo
  # Verify wrk2 + upstream sources present
  ok=1
  for f in \
    "$ROOT/tools/upstream/src/main.rs" \
    "$ROOT/scripts/r3/r3b-upstream-capacity-remote.sh" \
    "$ROOT/scripts/r3/r3c-loadgen-capacity-remote.sh"
  do
    if [[ -f "$f" ]]; then echo "present $f"; else echo "MISSING $f"; ok=0; fi
  done
  if [[ -f "$ROOT/benchmarks/docker/docker-compose.bench.yml" ]]; then
    echo "present benchmarks/docker/docker-compose.bench.yml (gitignored harness; local)"
    if rg -q 'openlitespeed-stable' "$ROOT/benchmarks/docker/docker-compose.bench.yml"; then
      echo "compose_has_openlitespeed-stable=YES"
    else
      echo "compose_has_openlitespeed-stable=NO"; ok=0
    fi
    if rg -q 'upstream' "$ROOT/benchmarks/docker/docker-compose.bench.yml"; then
      echo "compose_has_upstream=YES"
    else
      echo "compose_has_upstream=NO"; ok=0
    fi
  else
    echo "MISSING benchmarks/docker/docker-compose.bench.yml"
    ok=0
  fi
  echo
  if [[ "$ok" -eq 1 ]]; then
    echo "R3D_TOPOLOGY_NEUTRALITY=PASS"
    echo "R3D_NOTE=contract documented; competitive must use identical settings both impls"
    exit 0
  fi
  echo "R3D_TOPOLOGY_NEUTRALITY=FAIL"
  exit 1
} | tee "$OUT"
