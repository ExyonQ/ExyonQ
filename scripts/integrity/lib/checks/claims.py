#!/usr/bin/env python3
"""Check 8 — feature claim traceability."""
from __future__ import annotations

import re
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

from context import ScanContext, read_text
from model import CLAIM_STATUSES, ClaimRecord, Finding

try:
    import tomllib  # Python 3.11+
except ModuleNotFoundError:  # pragma: no cover
    tomllib = None  # type: ignore

DEFAULT_CLAIMS = (
    "docs/governance/integrity/feature-claims.toml",
    "scripts/integrity/claims/feature-claims.toml",
)


def _loads_toml(text: str) -> Dict[str, Any]:
    if tomllib is not None:
        return tomllib.loads(text)
    # Minimal fallback for [[claim]] tables (stdlib-only on older Python).
    return _minimal_claims_toml(text)


def _minimal_claims_toml(text: str) -> Dict[str, Any]:
    claims: List[Dict[str, Any]] = []
    cur: Optional[Dict[str, Any]] = None
    evidence_mode = False
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line == "[[claim]]":
            if cur:
                claims.append(cur)
            cur = {"evidence": []}
            evidence_mode = False
            continue
        if cur is None:
            continue
        if line.startswith("evidence") and "=" in line and "[" in line:
            evidence_mode = True
            if line.rstrip().endswith("]"):
                # inline empty or single-line array
                inner = line.split("=", 1)[1].strip()
                cur["evidence"] = _parse_str_array(inner)
                evidence_mode = False
            continue
        if evidence_mode:
            if line.startswith("]"):
                evidence_mode = False
                continue
            m = re.match(r'^"([^"]*)"\s*,?$', line)
            if m:
                cur.setdefault("evidence", []).append(m.group(1))
            continue
        m = re.match(r'^([A-Za-z0-9_]+)\s*=\s*"(.*)"\s*$', line)
        if m:
            cur[m.group(1)] = m.group(2)
    if cur:
        claims.append(cur)
    return {"claim": claims}


def _parse_str_array(inner: str) -> List[str]:
    inner = inner.strip()
    if inner == "[]":
        return []
    return re.findall(r'"([^"]*)"', inner)


def _parse_claims(path: Path) -> Tuple[List[ClaimRecord], List[str]]:
    errors: List[str] = []
    try:
        data = _loads_toml(path.read_text(encoding="utf-8"))
    except Exception as exc:  # noqa: BLE001
        return [], [f"TOML parse error: {exc}"]

    records: List[ClaimRecord] = []
    items = data.get("claim") or data.get("claims") or []
    if isinstance(data.get("claims"), dict):
        # alternate map form
        items = [{"id": k, **v} for k, v in data["claims"].items()]
    if not isinstance(items, list):
        return [], ["claims must be an array of tables [[claim]]"]

    for item in items:
        if not isinstance(item, dict):
            errors.append("non-table claim entry")
            continue
        cid = str(item.get("id", "")).strip()
        status = str(item.get("status", "UNVERIFIED")).strip().upper()
        if not cid:
            errors.append("claim missing id")
            continue
        if status not in CLAIM_STATUSES:
            errors.append(f"{cid}: invalid status {status}")
            continue
        evidence = item.get("evidence") or []
        if isinstance(evidence, str):
            evidence = [evidence]
        records.append(
            ClaimRecord(
                id=cid,
                description=str(item.get("description", "")),
                status=status,
                entrypoint=str(item.get("entrypoint", "")),
                evidence=[str(e) for e in evidence],
                notes=str(item.get("notes", "")),
            )
        )
    return records, errors


def _evidence_exists(root: Path, token: str) -> Tuple[bool, str]:
    """Evidence tokens: path:REL, test:REL, command:TEXT, gate:REL."""
    if ":" not in token:
        p = root / token
        return p.exists(), f"path-exists={p.exists()}"
    kind, _, rest = token.partition(":")
    kind = kind.strip().lower()
    rest = rest.strip()
    if kind in {"path", "test", "gate", "file"}:
        p = root / rest
        return p.exists(), f"{kind} exists={p.exists()} -> {rest}"
    if kind == "command":
        # Commands are not executed here — only recorded as UNVERIFIED unless listed
        return True, "command declared (not executed by static auditor)"
    if kind == "rg" or kind == "symbol":
        # symbol:Name — search if symbol appears in tree (best-effort)
        hit = False
        # limited scan
        from context import iter_files, rel_of

        for path in iter_files(root, None):
            rel = rel_of(root, path)
            if not rel.endswith(".rs"):
                continue
            if rest in read_text(path):
                hit = True
                break
        return hit, f"symbol_search={hit}"
    return False, f"unknown evidence kind '{kind}'"


def run(ctx: ScanContext) -> List[Finding]:
    findings: List[Finding] = []
    claims_path = None
    for rel in DEFAULT_CLAIMS:
        candidate = ctx.root / rel
        if candidate.is_file():
            claims_path = candidate
            break

    if claims_path is None:
        findings.append(
            Finding(
                id=ctx.next_id("CLAIM"),
                severity="REVIEW",
                category="FEATURE_CLAIM_TRACEABILITY",
                path="docs/governance/integrity/feature-claims.toml",
                line=0,
                claim="Canonical feature-claims.toml exists",
                evidence="missing claims file at documented locations",
                why_it_matters="Without claims, feature completeness cannot be traced to evidence.",
                confidence="HIGH",
                recommended_action="Create docs/governance/integrity/feature-claims.toml",
                classification="REVIEW_REQUIRED",
                check="claims",
            )
        )
        return findings

    records, errors = _parse_claims(claims_path)
    rel = str(claims_path.relative_to(ctx.root))
    for err in errors:
        findings.append(
            Finding(
                id=ctx.next_id("CLAIM"),
                severity="HIGH",
                category="FEATURE_CLAIM_TRACEABILITY",
                path=rel,
                line=1,
                claim="Claims file is well-formed",
                evidence=err,
                why_it_matters="Malformed claims hide verification state.",
                confidence="HIGH",
                recommended_action="Fix TOML / status enum.",
                classification="REVIEW_REQUIRED",
                check="claims",
            )
        )

    # Attach to report via side channel: store on ctx dynamically
    ctx.claims = records  # type: ignore[attr-defined]

    for rec in records:
        if rec.status == "VERIFIED":
            if not rec.evidence:
                findings.append(
                    Finding(
                        id=ctx.next_id("CLAIM"),
                        severity="CRITICAL",
                        category="FEATURE_CLAIM_TRACEABILITY",
                        path=rel,
                        line=1,
                        claim=f"VERIFIED claim {rec.id} includes evidence",
                        evidence="status=VERIFIED but evidence=[]",
                        why_it_matters="CLAIM != EVIDENCE — verified without pointers is hallucination risk.",
                        confidence="HIGH",
                        recommended_action="Attach path:/test:/command: evidence or downgrade status.",
                        classification="CRITICAL_INTEGRITY_VIOLATION",
                        check="claims",
                    )
                )
                continue
            missing = []
            for ev in rec.evidence:
                ok, detail = _evidence_exists(ctx.root, ev)
                if not ok:
                    missing.append(f"{ev} ({detail})")
            if missing:
                findings.append(
                    Finding(
                        id=ctx.next_id("CLAIM"),
                        severity="HIGH",
                        category="FEATURE_CLAIM_TRACEABILITY",
                        path=rel,
                        line=1,
                        claim=f"Evidence for VERIFIED claim {rec.id} exists",
                        evidence="; ".join(missing)[:400],
                        why_it_matters="Referenced evidence must exist for VERIFIED status.",
                        confidence="HIGH",
                        recommended_action="Fix evidence paths or set status PARTIAL/UNVERIFIED.",
                        classification="PRODUCT_PATH_FAKE",
                        check="claims",
                    )
                )
        elif rec.status == "NOT_IMPLEMENTED":
            findings.append(
                Finding(
                    id=ctx.next_id("CLAIM"),
                    severity="INFO",
                    category="FEATURE_CLAIM_TRACEABILITY",
                    path=rel,
                    line=1,
                    claim=f"Claim {rec.id} explicitly NOT_IMPLEMENTED",
                    evidence=rec.description[:200],
                    why_it_matters="Explicit gap is healthier than implied completeness.",
                    confidence="HIGH",
                    recommended_action="Keep status until real evidence exists.",
                    classification="REVIEW_REQUIRED",
                    check="claims",
                )
            )
        elif rec.status in {"UNVERIFIED", "PARTIAL", "DEFERRED", "BROKEN"}:
            sev = "HIGH" if rec.status == "BROKEN" else "REVIEW"
            findings.append(
                Finding(
                    id=ctx.next_id("CLAIM"),
                    severity=sev,
                    category="FEATURE_CLAIM_TRACEABILITY",
                    path=rel,
                    line=1,
                    claim=f"Claim {rec.id} status={rec.status}",
                    evidence=rec.description[:200] or rec.id,
                    why_it_matters="Non-verified claims must not be treated as PASS.",
                    confidence="HIGH",
                    recommended_action="Collect executable evidence before raising status.",
                    classification="REVIEW_REQUIRED",
                    check="claims",
                )
            )

        if rec.entrypoint:
            ep = ctx.root / rec.entrypoint
            if not ep.exists():
                findings.append(
                    Finding(
                        id=ctx.next_id("CLAIM"),
                        severity="MEDIUM",
                        category="FEATURE_CLAIM_TRACEABILITY",
                        path=rel,
                        line=1,
                        claim=f"Entrypoint for {rec.id} exists",
                        evidence=f"missing entrypoint path: {rec.entrypoint}",
                        why_it_matters="Broken entrypoint links undermine claim traceability.",
                        confidence="HIGH",
                        recommended_action="Fix entrypoint path.",
                        classification="REVIEW_REQUIRED",
                        check="claims",
                    )
                )
    return findings
