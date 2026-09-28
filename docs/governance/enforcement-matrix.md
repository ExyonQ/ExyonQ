# R5 blocker control enforcement matrix

```text
DOCUMENT = docs/governance/enforcement-matrix.md
ROLE = POLICY_ENFORCEMENT_MAP
GRC_P1_STARTED = NO
```

Levels: `POLICY_ONLY` · `MANUAL` · `AUTOMATED_LOCAL` · `AUTOMATED_CI` · `RELEASE_GATE`

| Control | Policy | Enforcement | Level |
|---------|--------|-------------|-------|
| docs/security SoT tracked | SECURITY.md + docs/security/README | `.gitignore` exceptions + `git ls-files` + pre-R5 gate | AUTOMATED_LOCAL + RELEASE_GATE |
| docs/governance SoT tracked | docs/governance/README | `.gitignore` exceptions + pre-R5 gate | AUTOMATED_LOCAL + RELEASE_GATE |
| Release audit executable | docs/security/audit-template + checklist | `scripts/verify-release-audit.sh` (+ `release.yml` on tag) | AUTOMATED_LOCAL + RELEASE_GATE |
| Project integrity | docs/governance/project-integrity.md | `scripts/gates/project-integrity-gate.sh` | AUTOMATED_LOCAL (+ CI job) |
| Benchmark integrity | docs/governance/benchmark-integrity.md | `scripts/gates/benchmark-integrity-gate.sh` | AUTOMATED_LOCAL (+ CI job) |
| Data provenance | docs/governance/data-provenance.md | `scripts/gates/data-provenance-gate.sh` | AUTOMATED_LOCAL (+ CI selftest) |
| PMZ | docs/security/private-material-zero.md | `scan-private-material.sh` + `.githooks` + `security.yml` | AUTOMATED_LOCAL + AUTOMATED_CI + RELEASE_GATE |
| Git hooks coherent | setup-githooks.sh | tracked `.githooks/` + `core.hooksPath=.githooks` | AUTOMATED_LOCAL |
| Waiver register | docs/security/waiver-requirements.md | `docs/governance/exceptions/` + `waiver-expiry-gate.sh` | AUTOMATED_LOCAL + RELEASE_GATE |
| Evidence binder | docs/governance/evidence/INDEX.md | pre-R5 gate presence check | MANUAL (binder) + AUTOMATED_LOCAL (presence) |
| Signing governance | docs/governance/signing-readiness.md + security/signing | verify scripts; **no** production sign in CI by default | POLICY + MANUAL ceremony |
| No smoke as proof | rule 127 + PROJECT_STATUS | `no-smoke-as-proof-gate.sh` | AUTOMATED_LOCAL |

```text
R5_POLICY_WITHOUT_REQUIRED_ENFORCEMENT = 0
R5_ENFORCEMENT_WITHOUT_CANONICAL_POLICY = 0
```

Signing production execution remains MANUAL owner ceremony (`PRODUCTION_SIGNING_EXECUTED=NO`).
