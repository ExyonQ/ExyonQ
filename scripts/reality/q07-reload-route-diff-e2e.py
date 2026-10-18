#!/usr/bin/env python3
"""Q-07 / ADR-056 — reload --diff prints routes_added/removed + fingerprint (offline)."""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
HEAD = os.environ.get("HEAD", "UNKNOWN")
CTL = Path(os.environ.get("EXYONQCTL", str(WS / "target" / "debug" / "exyonqctl")))

BASE = """config_version = 1
[[server]]
listen = "127.0.0.1:19081"
routes = ["home"]
[[route]]
name = "home"
match = { path = "/" }
root = "/tmp"
"""

CAND = """config_version = 1
[[server]]
listen = "127.0.0.1:19081"
routes = ["home", "api"]
[[route]]
name = "home"
match = { path = "/" }
root = "/tmp"
[[route]]
name = "api"
match = { path = "/api" }
root = "/tmp"
"""


def main() -> int:
    if not CTL.is_file():
        print(f"FAIL: missing {CTL}", file=sys.stderr)
        return 2
    EV.mkdir(parents=True, exist_ok=True)
    td = Path(tempfile.mkdtemp(prefix="q07-", dir=str(EV)))
    base = td / "base.toml"
    cand = td / "cand.toml"
    base.write_text(BASE, encoding="utf-8")
    cand.write_text(CAND, encoding="utf-8")
    proc = subprocess.run(
        [
            str(CTL),
            "config",
            "reload",
            str(cand),
            "--diff",
            "--baseline",
            str(base),
        ],
        cwd=str(WS),
        capture_output=True,
        text=True,
        env={**os.environ, "EXYONQ_CONFIG": str(cand)},
    )
    combined = (proc.stdout or "") + (proc.stderr or "")
    (EV / "exyonqctl.out").write_text(combined, encoding="utf-8")
    checks = {
        "exit_ok": {"ok": proc.returncode == 0},
        "fingerprint": {"ok": "candidate_fingerprint =" in combined},
        "routes_added": {
            "ok": "routes_added = api@*:prefix:/api" in combined
            or "api@*:prefix:/api" in combined
        },
        "routes_removed": {"ok": "routes_removed = (none)" in combined},
    }
    verdict = "PASS" if all(c["ok"] for c in checks.values()) else "FAIL"
    result = {
        "slice": "Q-07",
        "adr": "ADR-056",
        "head": HEAD,
        "started_utc": datetime.now(timezone.utc).isoformat(),
        "exit": proc.returncode,
        "checks": checks,
        "verdict": verdict,
    }
    OUT.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"verdict": verdict, "out": str(OUT)}))
    return 0 if verdict == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
