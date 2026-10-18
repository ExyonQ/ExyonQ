#!/usr/bin/env python3
"""Q-03 / ADR-055 — Require all denied in .htaccess fails closed (no silent publish)."""
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


def main() -> int:
    if not CTL.is_file():
        print(f"FAIL: missing {CTL}", file=sys.stderr)
        return 2
    EV.mkdir(parents=True, exist_ok=True)
    td = Path(tempfile.mkdtemp(prefix="q03-ht-", dir=str(EV)))
    (td / ".htaccess").write_text(
        "Require all denied\nRedirect 301 /old /new\n", encoding="utf-8"
    )
    proc = subprocess.run(
        [str(CTL), "config", "htaccess", "compile", str(td), "--site", "q03"],
        cwd=str(WS),
        capture_output=True,
        text=True,
    )
    combined = (proc.stdout or "") + (proc.stderr or "")
    (EV / "exyonqctl.out").write_text(combined, encoding="utf-8")
    has_code = "EXY-HTACCESS-0003" in combined
    fail_closed = proc.returncode != 0
    result = {
        "slice": "Q-03",
        "adr": "ADR-055",
        "head": HEAD,
        "started_utc": datetime.now(timezone.utc).isoformat(),
        "exit": proc.returncode,
        "checks": {
            "nonzero_exit": {"ok": fail_closed},
            "exy_htaccess_0003": {"ok": has_code, "excerpt": combined[:2000]},
        },
        "verdict": "PASS" if fail_closed and has_code else "FAIL",
    }
    OUT.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"verdict": result["verdict"], "out": str(OUT)}))
    return 0 if result["verdict"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
