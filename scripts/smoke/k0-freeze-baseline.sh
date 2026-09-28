#!/usr/bin/env bash
# Pull K0.5 remote baseline artifacts and freeze internal non-publishable run_meta.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SSH_OPTS="${K0_SSH_OPTS:--o BatchMode=yes -o ServerAliveInterval=30}"
FROZEN="${K0_FROZEN_DIR:-$ROOT/benchmarks/results-dev/k0.5-baseline-frozen}"
mkdir -p "$FROZEN"

NETCUP_SSH="${NETCUP_SSH:-netcup-bench}"
ORACLE_SSH="${ORACLE_SSH:-oracle-quasar}"
NETCUP_SOAK="${NETCUP_SOAK:-/root/exyonq-dev-soak/k0.5-baseline}"
ORACLE_SOAK="${ORACLE_SOAK:-/home/ubuntu/exyonq-dev-soak/k0.5-baseline}"

pull_host() {
  local label="$1" ssh_host="$2" soak_root="$3"
  local latest remote_dir local_dir
  latest=$(ssh $SSH_OPTS "$ssh_host" "ls -td ${soak_root}/*/ 2>/dev/null | head -1" || true)
  [[ -n "$latest" ]] || { echo "missing K0.5 run on $label ($soak_root)" >&2; return 1; }
  remote_dir="${latest%/}"
  local_dir="$FROZEN/${label}"
  mkdir -p "$local_dir"
  rsync -az -e "ssh $SSH_OPTS" "${ssh_host}:${remote_dir}/" "$local_dir/"
  echo "$local_dir"
}

fix_summaries() {
  local host_dir="$1"
  for sid in p1 p2 p3 p4 health metrics; do
    local sdir="$host_dir/$sid"
    [[ -d "$sdir" ]] || continue
    python3 "$ROOT/scripts/smoke/k0_resource_summary.py" "$sdir" "$sid" >/dev/null
  done
}

rebuild_host_meta() {
  local label="$1" host_dir="$2"
  python3 - "$label" "$host_dir" <<'PY'
import json
import sys
from pathlib import Path

label, host_dir = sys.argv[1], Path(sys.argv[2])
rows = []
for sid in ("p1", "p2", "p3", "p4", "health", "metrics"):
    sdir = host_dir / sid
    summary = sdir / "summary.json"
    if not summary.is_file():
        continue
    s = json.loads(summary.read_text())
    rows.append({
        "id": sid,
        "rps": s.get("rps"),
        "p50_ms": s.get("p50_ms"),
        "p95_ms": s.get("p95_ms"),
        "p99_ms": s.get("p99_ms"),
        "ok": s.get("ok"),
        "fail": s.get("fail"),
        "success_rate": s.get("success_rate"),
        "cpu_peak_pct": s.get("cpu_peak_pct"),
        "rss_peak_mib": s.get("rss_peak_mib"),
        "rss_peak_bytes": s.get("rss_peak_bytes"),
    })

orig = {}
if (host_dir / "run_meta.json").is_file():
    orig = json.loads((host_dir / "run_meta.json").read_text())

metrics = {}
if (host_dir / "metrics-final.txt").is_file():
    for line in (host_dir / "metrics-final.txt").read_text().splitlines():
        if line.startswith("exyonq_static_sendfile_"):
            k, _, v = line.partition(" ")
            metrics[k] = int(v)

meta = {
    **{k: v for k, v in orig.items() if k not in ("scenarios", "gate")},
    "k0_5": True,
    "exyonq_only": True,
    "official": False,
    "official_claim": False,
    "publishable": False,
    "internal_baseline": True,
    "baseline_id": "k0.5-baseline-frozen",
    "host": label,
    "load_tool": "protector_loadgen_asyncio",
    "scenarios": rows,
    "metrics_final": metrics,
    "gate": (host_dir / "gate.txt").read_text().strip() if (host_dir / "gate.txt").is_file() else orig.get("gate"),
}
(host_dir / "run_meta.json").write_text(json.dumps(meta, indent=2) + "\n")
print(json.dumps(meta, indent=2))
PY
}

echo "K0.5 freeze -> $FROZEN"
NETCUP_DIR=$(pull_host "netcup-amd64" "$NETCUP_SSH" "$NETCUP_SOAK")
ORACLE_DIR=$(pull_host "oracle-aarch64" "$ORACLE_SSH" "$ORACLE_SOAK")

fix_summaries "$NETCUP_DIR"
fix_summaries "$ORACLE_DIR"

rebuild_host_meta "netcup-amd64" "$NETCUP_DIR" >"$FROZEN/run_meta.netcup-amd64.json"
rebuild_host_meta "oracle-aarch64" "$ORACLE_DIR" >"$FROZEN/run_meta.oracle-aarch64.json"

python3 - "$FROZEN" "$NETCUP_DIR" "$ORACLE_DIR" <<'PY'
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

frozen, netcup, oracle = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
net_meta = json.loads((frozen / "run_meta.netcup-amd64.json").read_text())
ora_meta = json.loads((frozen / "run_meta.oracle-aarch64.json").read_text())

merged = {
    "baseline_id": "k0.5-baseline-frozen",
    "k0_5": True,
    "frozen_at_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "official": False,
    "official_claim": False,
    "publishable": False,
    "internal_baseline": True,
    "methodology_note": "ExyonQ-only protector asyncio loadgen; NOT comparable to K0.6 ReWrk compare",
    "hosts": {
        "netcup-amd64": {"dir": str(netcup.name), "gate": net_meta.get("gate"), "commit": net_meta.get("commit")},
        "oracle-aarch64": {"dir": str(oracle.name), "gate": ora_meta.get("gate"), "commit": ora_meta.get("commit")},
    },
    "scenarios_by_host": {
        "netcup-amd64": net_meta.get("scenarios", []),
        "oracle-aarch64": ora_meta.get("scenarios", []),
    },
}
(frozen / "run_meta.k0.5-frozen.json").write_text(json.dumps(merged, indent=2) + "\n")
(frozen / "run_meta.json").write_text(json.dumps(merged, indent=2) + "\n")
print(f"frozen: {frozen / 'run_meta.json'}")
PY

echo "K0.5 baseline frozen at $FROZEN"
