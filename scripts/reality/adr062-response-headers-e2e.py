#!/usr/bin/env python3
"""ADR-062 transition pack — response_headers + rewrite $uri/$host proof."""

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
        / ".exyonq-local/evidence/adr062-response-headers"
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

    checks_spec = [
        ("ir_response_headers", ["-p", "exyonq-config-ir", "response_headers_"]),
        ("ir_rewrite_uri_host", ["-p", "exyonq-config-ir", "rewrite_allows_uri"]),
        ("rewrite_engine_vars", ["-p", "exyonq-module-api", "uri_and_host_vars"]),
        ("nginx_add_header", ["-p", "exyonq-compat-nginx", "tier2b_add_header"]),
        ("apache_header_set", ["-p", "exyonq-compat-apache", "migrates_header_set"]),
        # --lib: avoid compiling every core integration binary for a unit filter
        (
            "runtime_status_gate",
            [
                "-p",
                "exyonq-core",
                "--lib",
                "apply_route_response_headers_status_gate",
            ],
        ),
    ]
    for name, args in checks_spec:
        p = run(["cargo", "test", *args, "--", "--nocapture"])
        checks[name] = {
            "ok": p.returncode == 0 and "ok" in p.stdout,
            "rc": p.returncode,
            "stdout_tail": (p.stdout + p.stderr)[-800:],
        }

    verdict = "PASS" if all(c.get("ok") for c in checks.values()) else "FAIL"
    result = {
        "slice": "ADR-062-transition-pack",
        "adr": "ADR-062",
        "residuals": ["F1-R19", "F1-R07", "F1-R15-narrow"],
        "head": HEAD,
        "started_utc": datetime.now(timezone.utc).isoformat(),
        "checks": checks,
        "verdict": verdict,
    }
    OUT_JSON.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    (EV_DIR / "PHASE_ADR062.md").write_text(
        f"# ADR-062 transition pack\n\nVerdict: **{verdict}**\n\n"
        f"Evidence: `{OUT_JSON}`\n",
        encoding="utf-8",
    )
    print(json.dumps({"verdict": verdict, "out": str(OUT_JSON)}))
    return 0 if verdict == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
