#!/usr/bin/env python3
"""Root-cause aggregation: UNIQUE_ROOT_CAUSES vs INDIVIDUAL_OCCURRENCES."""
from __future__ import annotations

from collections import defaultdict
from typing import Any, Dict, List, Tuple

from model import Finding


def root_cause_key(f: Finding) -> Tuple[str, str, str]:
    """Group by classification + check + path family (dir or stem pattern)."""
    path = f.path.replace("\\", "/")
    if "/smoke/" in path or path.startswith("scripts/smoke"):
        family = "scripts/smoke/**"
    elif path.startswith("scripts/e2e/"):
        family = "scripts/e2e/**"
    elif "exyonq-mod-fastcgi" in path:
        family = "crates/exyonq-mod-fastcgi/**"
    elif path.startswith("benchmarks/scenarios/perf/"):
        family = "benchmarks/scenarios/perf/**"
    elif path.startswith("tools/upstream"):
        family = "tools/upstream/**"
    elif path.startswith("scripts/r3/"):
        family = "scripts/r3/**"
    elif "/tests/" in path or path.endswith("_test.rs"):
        parts = path.split("/")
        family = "/".join(parts[:2]) + "/**/tests/**" if len(parts) >= 2 else path
    else:
        # directory family
        family = "/".join(path.split("/")[:3]) + "/**" if path.count("/") >= 2 else path
    return (f.classification or "UNCLASSIFIED", f.check or "unknown", family)


def aggregate_root_causes(findings: List[Finding]) -> Dict[str, Any]:
    groups: Dict[Tuple[str, str, str], List[Finding]] = defaultdict(list)
    for f in findings:
        if f.severity == "INFO" and f.classification == "REAL_TEST_INPUT":
            continue
        groups[root_cause_key(f)].append(f)

    rows = []
    for (klass, check, family), items in sorted(
        groups.items(),
        key=lambda kv: (_worst_rank(kv[1]), kv[0][0], kv[0][2]),
    ):
        worst = min(items, key=lambda x: _sev_rank(x.severity))
        paths = sorted({i.path for i in items})[:12]
        # Prefer FORBIDDEN_* groups; skip pure INFO observational clusters from top table
        if worst.severity == "INFO" and not str(klass).startswith("FORBIDDEN_"):
            continue
        rows.append(
            {
                "id": f"RC-{klass[:16]}-{check}-{abs(hash(family)) % 10000:04d}",
                "severity": worst.severity,
                "type": klass,
                "zone_hint": family,
                "paths": paths,
                "path_count": len({i.path for i in items}),
                "occurrence_count": len(items),
                "evidence": worst.evidence[:200],
                "recommended_action": worst.recommended_action,
                "check": check,
            }
        )
    return {
        "UNIQUE_ROOT_CAUSES": len(rows),
        "INDIVIDUAL_OCCURRENCES": len(findings),
        "root_causes": rows,
    }


def _sev_rank(sev: str) -> int:
    order = {"CRITICAL": 0, "HIGH": 1, "MEDIUM": 2, "REVIEW": 3, "LOW": 4, "INFO": 5}
    return order.get(sev, 9)


def _worst_rank(items: List[Finding]) -> int:
    return min(_sev_rank(i.severity) for i in items)
