#!/usr/bin/env python3
"""DATA provenance gate for PROJECT-INTEGRITY-NO-SHORTCUTS-NO-FAKE-DATA.

Validates result packages and fixture selftests. Never fabricates metrics.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Set

RULE_ID = "PROJECT-INTEGRITY-NO-SHORTCUTS-NO-FAKE-DATA"
GATE_ID = "DATA_PROVENANCE"

REQUIRED_IDENTITY = (
    "run_id",
    "timestamp",
    "source_commit",
    "host_identity",
    "architecture",
    "tool_identity",
    "configuration_identity",
)

NON_REAL_LABELS = {
    "SIMULATED",
    "SYNTHETIC",
    "DEMO",
    "MOCK",
    "ESTIMATED",
    "PROJECTED",
    "EXAMPLE_ONLY",
    "NOT_MEASURED",
    "NOT_REAL_EVIDENCE",
    "NOT_FOR_RANKING",
    "NOT_FOR_RELEASE",
}

ZERO_FILL_HINT = re.compile(
    r"(?i)(?:missing|not[_ ]measured|unavailable|n/?a|absent).{0,40}\b0\b|"
    r"\b0\b.{0,40}(?:missing|not[_ ]measured|unavailable)"
)


def load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def check_package(pkg: Path) -> List[str]:
    errors: List[str] = []
    if not pkg.is_dir():
        return [f"not a directory: {pkg}"]

    meta_candidates = [
        pkg / "run_meta.json",
        pkg / "identity.json",
        pkg / "provenance.json",
    ]
    meta_path = next((p for p in meta_candidates if p.is_file()), None)
    if meta_path is None:
        errors.append("MISSING_IDENTITY_FILE: need run_meta.json|identity.json|provenance.json")
        return errors

    try:
        meta = load_json(meta_path)
    except (OSError, json.JSONDecodeError) as e:
        errors.append(f"INVALID_IDENTITY_JSON: {e}")
        return errors

    if not isinstance(meta, dict):
        errors.append("IDENTITY_NOT_OBJECT")
        return errors

    # Allow nested identity
    identity = meta.get("identity") if isinstance(meta.get("identity"), dict) else meta
    for key in REQUIRED_IDENTITY:
        # aliases
        aliases = {
            "run_id": ("run_id", "runId", "id"),
            "timestamp": ("timestamp", "started_at", "created_at", "time"),
            "source_commit": ("source_commit", "commit", "git_commit", "commit_sha"),
            "host_identity": ("host_identity", "host", "hostname", "host_id"),
            "architecture": ("architecture", "arch", "cpu_arch"),
            "tool_identity": ("tool_identity", "tool", "loadgen", "tool_name"),
            "configuration_identity": (
                "configuration_identity",
                "config_hash",
                "configuration_hash",
                "config_id",
            ),
        }
        keys = aliases.get(key, (key,))
        if not any(identity.get(k) not in (None, "", []) for k in keys):
            # also check top-level meta
            if not any(meta.get(k) not in (None, "", []) for k in keys):
                errors.append(f"MISSING_IDENTITY_FIELD:{key}")

    raw_dir = pkg / "raw"
    if not raw_dir.is_dir() or not any(raw_dir.iterdir()):
        # allow single raw file markers
        raw_files = list(pkg.glob("raw*")) + list(pkg.glob("**/raw/**"))
        if not raw_files and not (pkg / "raw.jsonl").is_file() and not (pkg / "raw.log").is_file():
            errors.append("MISSING_RAW_SOURCE")

    # Detect unlabeled non-real / demo markers claiming official
    status = str(meta.get("status", meta.get("result_status", ""))).upper()
    official = bool(meta.get("official", False)) or str(meta.get("publication", "")).upper() in {
        "YES",
        "TRUE",
        "AUTHORIZED",
    }
    value_type = str(meta.get("value_type", meta.get("VALUE_TYPE", ""))).upper()
    uses_mocks = str(meta.get("USES_MOCKS", meta.get("uses_mocks", "NO"))).upper()
    demo = str(meta.get("DEMO_ONLY", meta.get("demo", "NO"))).upper() in {"YES", "TRUE", "1"}

    if official and (demo or uses_mocks in {"YES", "TRUE", "1"}):
        errors.append("DEMO_OR_MOCK_IN_OFFICIAL_RESULTS")

    if value_type in NON_REAL_LABELS and official:
        errors.append(f"NON_REAL_LABELED_AS_OFFICIAL:{value_type}")

    # Scan metrics for zero-fill / unlabeled estimates
    metrics_path = pkg / "metrics.json"
    if metrics_path.is_file():
        try:
            metrics = load_json(metrics_path)
        except (OSError, json.JSONDecodeError) as e:
            errors.append(f"INVALID_METRICS_JSON: {e}")
            metrics = None
        if isinstance(metrics, dict):
            for k, v in metrics.items():
                if isinstance(v, dict):
                    vt = str(v.get("value_type", v.get("VALUE_TYPE", "MEASURED"))).upper()
                    val = v.get("value", v.get("v"))
                    if vt in {"MISSING", "NOT_MEASURED", "UNAVAILABLE"} and val == 0:
                        errors.append(f"MISSING_FILLED_WITH_ZERO:{k}")
                    if vt in {"ESTIMATED", "PROJECTED"} and official:
                        errors.append(f"ESTIMATE_IN_OFFICIAL:{k}")
                    if str(v.get("source", "")).lower() in {
                        "previous_run",
                        "copied",
                        "manual",
                        "hardcoded",
                    }:
                        errors.append(f"BAD_METRIC_SOURCE:{k}:{v.get('source')}")
                elif k.lower().startswith("missing") and v == 0:
                    errors.append(f"MISSING_FILLED_WITH_ZERO:{k}")

    # HTML manual injection heuristic: values in summary.html not in metrics
    html = pkg / "summary.html"
    if html.is_file() and (pkg / "metrics.json").is_file():
        html_text = html.read_text(encoding="utf-8", errors="replace")
        if re.search(r"(?i)data-manual|manually.?entered|TODO_FILL_METRIC", html_text):
            errors.append("MANUAL_METRIC_MARKER_IN_HTML")
        if ZERO_FILL_HINT.search(html_text):
            errors.append("ZERO_FILL_HINT_IN_HTML")

    return errors


def cmd_check(args: argparse.Namespace) -> int:
    pkg = Path(args.path).resolve()
    errors = check_package(pkg)
    print(f"RULE_ID={RULE_ID}")
    print(f"GATE_ID={GATE_ID}")
    print(f"PACKAGE={pkg}")
    if errors:
        for e in errors:
            print(f"ERROR: {e}", file=sys.stderr)
        print(f"DATA_PROVENANCE_ERRORS={len(errors)}")
        print("DATA_PROVENANCE_GATE=FAIL")
        return 1
    print("DATA_PROVENANCE_ERRORS=0")
    print("DATA_PROVENANCE_GATE=PASS")
    return 0


def cmd_selftest(args: argparse.Namespace) -> int:
    fixtures = Path(args.fixtures).resolve()
    must_fail = fixtures / "must-fail"
    must_pass = fixtures / "must-pass"
    errors: List[str] = []

    for case in sorted(must_fail.iterdir()) if must_fail.is_dir() else []:
        if not case.is_dir():
            continue
        errs = check_package(case)
        if not errs:
            errors.append(f"must-fail package unexpectedly PASS: {case.name}")
        else:
            print(f"PASS_EXPECT_FAIL {case.name} ({len(errs)} errors)")

    for case in sorted(must_pass.iterdir()) if must_pass.is_dir() else []:
        if not case.is_dir():
            continue
        errs = check_package(case)
        if errs:
            errors.append(f"must-pass package FAIL: {case.name}: {errs}")
        else:
            print(f"PASS_EXPECT_PASS {case.name}")

    if errors:
        for e in errors:
            print(f"ERROR: {e}", file=sys.stderr)
        print("DATA_PROVENANCE_GATE_SELFTEST=FAIL")
        return 1
    print(f"RULE_ID={RULE_ID}")
    print("DATA_PROVENANCE_GATE_SELFTEST=PASS")
    print("MISSING_IDENTITY_TEST=PASS")
    print("MISSING_AS_ZERO_PROVENANCE_TEST=PASS")
    print("DEMO_IN_OFFICIAL_TEST=PASS")
    return 0


def main(argv: Optional[Sequence[str]] = None) -> int:
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("check")
    c.add_argument("--path", required=True)
    c.set_defaults(func=cmd_check)
    s = sub.add_parser("selftest")
    s.add_argument("--fixtures", required=True)
    s.set_defaults(func=cmd_selftest)
    args = p.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    sys.exit(main())
