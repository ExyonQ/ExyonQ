#!/usr/bin/env python3
"""F2 transversal / ADR-061 — ExpiresByType → cache_headers + Cache-Control proof."""

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
        / ".exyonq-local/evidence/f2-expires-cache-headers"
        / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ"),
    )
)
OUT_JSON = Path(os.environ.get("OUT_JSON", EV_DIR / "result.json"))
HEAD = os.environ.get("HEAD", "unknown")


def run(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, text=True, capture_output=True, cwd=WS)


def main() -> int:
    EV_DIR.mkdir(parents=True, exist_ok=True)
    checks: dict = {}

    p = run(
        [
            "cargo",
            "test",
            "-p",
            "exyonq-config-ir",
            "cache_headers_",
            "--",
            "--nocapture",
        ]
    )
    checks["ir_cache_headers"] = {
        "ok": p.returncode == 0 and "ok" in p.stdout,
        "rc": p.returncode,
        "stdout_tail": p.stdout[-500:],
    }

    p = run(
        [
            "cargo",
            "test",
            "-p",
            "exyonq-compat-apache",
            "expires",
            "--",
            "--nocapture",
        ]
    )
    checks["apache_expires_migrate"] = {
        "ok": p.returncode == 0 and "migrates_expires_by_type" in p.stdout,
        "rc": p.returncode,
        "stdout_tail": p.stdout[-600:],
    }

    p = run(
        [
            "cargo",
            "test",
            "-p",
            "exyonq-core",
            "--lib",
            "insert_cache_control",
            "--",
            "--nocapture",
        ]
    )
    checks["runtime_insert_cache_control"] = {
        "ok": p.returncode == 0 and "ok" in p.stdout,
        "rc": p.returncode,
        "stdout_tail": p.stdout[-500:],
    }

    verdict = "PASS" if all(c.get("ok") for c in checks.values()) else "FAIL"
    result = {
        "slice": "F2-transversal-expires",
        "adr": "ADR-061",
        "head": HEAD,
        "started_utc": datetime.now(timezone.utc).isoformat(),
        "checks": checks,
        "verdict": verdict,
    }
    OUT_JSON.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"verdict": verdict, "out": str(OUT_JSON)}))
    return 0 if verdict == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())
