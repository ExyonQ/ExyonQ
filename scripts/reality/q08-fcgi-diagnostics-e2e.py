#!/usr/bin/env python3
"""Q-08 / ADR-060 — FCGI saturation diagnostics + exyonq_fcgi_* proof."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

WS = Path(os.environ.get("WS", Path(__file__).resolve().parents[2]))
EV_DIR = Path(
    os.environ.get(
        "EV_DIR",
        WS
        / ".exyonq-local/evidence/q08-fcgi-diagnostics"
        / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ"),
    )
)
OUT_JSON = Path(os.environ.get("OUT_JSON", EV_DIR / "result.json"))
HEAD = os.environ.get("HEAD", "unknown")


def run(args: list[str], cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, text=True, capture_output=True, cwd=cwd or WS)


def main() -> int:
    EV_DIR.mkdir(parents=True, exist_ok=True)
    checks: dict = {}

    p = run(
        [
            "cargo",
            "test",
            "-p",
            "exyonq-mod-fastcgi",
            "--lib",
            "diagnostics::",
            "--",
            "--nocapture",
        ]
    )
    checks["diagnostics_unit"] = {
        "ok": p.returncode == 0 and "message_names_timeout_and_recommendation" in p.stdout,
        "rc": p.returncode,
        "stdout_tail": p.stdout[-800:],
        "stderr_tail": p.stderr[-400:],
    }

    p = run(
        [
            "cargo",
            "test",
            "-p",
            "exyonq-mod-fastcgi",
            "--lib",
            "append_fcgi_prometheus_emits_q08_series",
            "--",
            "--nocapture",
        ]
    )
    checks["prometheus_series"] = {
        "ok": p.returncode == 0 and "ok" in p.stdout,
        "rc": p.returncode,
        "stdout_tail": p.stdout[-600:],
        "stderr_tail": p.stderr[-400:],
    }

    p = run(
        [
            "cargo",
            "test",
            "-p",
            "exyonq-mod-fastcgi",
            "--lib",
            "checkout_wait_times_out_to_pool_busy",
            "--",
            "--nocapture",
        ]
    )
    checks["saturation_pool_busy"] = {
        "ok": p.returncode == 0 and "ok" in p.stdout,
        "rc": p.returncode,
        "stdout_tail": p.stdout[-600:],
        "stderr_tail": p.stderr[-400:],
    }

    # Confirm registration compile path (metrics crate links appender).
    p = run(["cargo", "check", "-p", "exyonq-metrics"])
    checks["metrics_registers_appender"] = {
        "ok": p.returncode == 0,
        "rc": p.returncode,
        "stderr_tail": p.stderr[-400:],
    }

    verdict = "PASS" if all(c.get("ok") for c in checks.values()) else "FAIL"
    result = {
        "slice": "Q-08",
        "adr": "ADR-060",
        "head": HEAD,
        "started_utc": datetime.now(timezone.utc).isoformat(),
        "checks": checks,
        "verdict": verdict,
        "notes": "Process-aggregate exyonq_fcgi_* series; per-pool labels deferred per ADR-060",
    }
    OUT_JSON.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"verdict": verdict, "out": str(OUT_JSON)}))
    return 0 if verdict == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())
