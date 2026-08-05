# Integrity gate fixtures

## project-integrity

Path: `scripts/gates/fixtures/project-integrity/`

| Suite | Purpose |
|-------|---------|
| `must-fail/` | Product-like snippets that must trip `PRODUCT_SEMANTIC_BRANCH` or `FAKE_RESULT_GENERATION` |
| `must-pass/` | Observational / declared / labeled cases that must not block |
| `must-review/` | Named features / demo-share / synthetic integration → owner review |

```bash
bash scripts/gates/project-integrity-gate.sh --selftest
```

## data-provenance

Path: `scripts/gates/fixtures/data-provenance/`

| Suite | Purpose |
|-------|---------|
| `must-fail/` | Packages missing identity/raw, zero-filled MISSING, demo-in-official |
| `must-pass/` | Complete identity + raw + measured metrics |

```bash
bash scripts/gates/data-provenance-gate.sh --selftest
bash scripts/gates/data-provenance-gate.sh --check <result-package-dir>
```

Policies: `docs/governance/project-integrity.md`.
