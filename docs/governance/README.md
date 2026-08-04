# Governance documentation (canonical SoT)

```text
DOCS_GOVERNANCE_ROLE = CANONICAL_TRACKED_SOT
TRACKING = force-tracked under /docs/governance/** (see .gitignore exceptions)
```

| Path | Purpose |
|------|---------|
| [project-integrity.md](project-integrity.md) | Project integrity policy |
| [benchmark-integrity.md](benchmark-integrity.md) | Benchmark integrity policy |
| [data-provenance.md](data-provenance.md) | Data provenance policy |
| [signing-readiness.md](signing-readiness.md) | Signing design readiness (no production sign) |
| [enforcement-matrix.md](enforcement-matrix.md) | Policy ↔ enforcement map |
| [exceptions/](exceptions/) | Release waiver register (minimal) |
| [evidence/INDEX.md](evidence/INDEX.md) | Evidence binder |

Gates:

```bash
bash scripts/gates/project-integrity-gate.sh --selftest
bash scripts/gates/benchmark-integrity-gate.sh --selftest
bash scripts/gates/data-provenance-gate.sh --selftest
bash scripts/gates/waiver-expiry-gate.sh --check
bash scripts/gates/pre-r5-grc-readiness-gate.sh
```
