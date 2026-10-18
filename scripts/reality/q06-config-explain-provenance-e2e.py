#!/usr/bin/env python3
"""Q-06 / ADR-059 — config explain provenance E2E (value + file:line + precedence)."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

WS = Path(os.environ.get("WS", Path(__file__).resolve().parents[2]))
EXYONQCTL = Path(os.environ.get("EXYONQCTL", WS / "target/debug/exyonqctl"))
EV_DIR = Path(
    os.environ.get(
        "EV_DIR",
        WS
        / ".exyonq-local/evidence/q06-config-explain-provenance"
        / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ"),
    )
)
OUT_JSON = Path(os.environ.get("OUT_JSON", EV_DIR / "result.json"))
HEAD = os.environ.get("HEAD", "unknown")


def run(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, text=True, capture_output=True)


def main() -> int:
    EV_DIR.mkdir(parents=True, exist_ok=True)
    checks: dict = {}
    with tempfile.TemporaryDirectory(prefix="q06-") as td:
        td_path = Path(td)
        frag = td_path / "frag.toml"
        frag.write_text(
            """
[[route]]
name = "extra"
match = { path = "/x" }
root = "/from-include"
""",
            encoding="utf-8",
        )
        main_toml = td_path / "main.toml"
        main_toml.write_text(
            """
config_version = 2
include = ["frag.toml"]

[[server]]
listen = "127.0.0.1:0"
routes = ["r", "extra"]

[[route]]
name = "r"
match = { path = "/" }
root = "/tmp/root"
""",
            encoding="utf-8",
        )

        # 1) root scalar
        p = run(
            [
                str(EXYONQCTL),
                "config",
                "explain",
                "--path",
                str(main_toml),
                "--pointer",
                "config_version",
                "--format",
                "json",
            ]
        )
        ok = p.returncode == 0 and '"value": 2' in p.stdout and '"kind": "root"' in p.stdout
        checks["root_scalar"] = {
            "ok": ok,
            "rc": p.returncode,
            "stdout": p.stdout,
            "stderr": p.stderr,
        }

        # 2) include-named route field
        p = run(
            [
                str(EXYONQCTL),
                "config",
                "explain",
                "--path",
                str(main_toml),
                "--pointer",
                "route[name=extra].root",
            ]
        )
        ok = (
            p.returncode == 0
            and "/from-include" in p.stdout
            and "origin.kind = include" in p.stdout
            and "frag.toml" in p.stdout
            and "origin.line =" in p.stdout
        )
        checks["include_route"] = {
            "ok": ok,
            "rc": p.returncode,
            "stdout": p.stdout,
            "stderr": p.stderr,
        }

        # 3) IR default
        p = run(
            [
                str(EXYONQCTL),
                "config",
                "explain",
                "--path",
                str(main_toml),
                "--pointer",
                "full_page_cache.enabled",
                "--format",
                "json",
            ]
        )
        ok = (
            p.returncode == 0
            and '"kind": "default"' in p.stdout
            and '"winner": true' in p.stdout
        )
        checks["default_layer"] = {
            "ok": ok,
            "rc": p.returncode,
            "stdout": p.stdout,
            "stderr": p.stderr,
        }

        # 4) lexicon still works without --path
        p = run([str(EXYONQCTL), "config", "explain", "route"])
        ok = p.returncode == 0 and "[[route]]" in p.stdout
        checks["lexicon_unchanged"] = {
            "ok": ok,
            "rc": p.returncode,
            "stdout": p.stdout[:400],
            "stderr": p.stderr,
        }

    verdict = "PASS" if all(c.get("ok") for c in checks.values()) else "FAIL"
    result = {
        "slice": "Q-06",
        "adr": "ADR-059",
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
