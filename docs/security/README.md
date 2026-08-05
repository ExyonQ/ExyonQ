# Security documentation (canonical SoT)

```text
DOCS_SECURITY_ROLE = CANONICAL_TRACKED_SOT
TRACKING = force-tracked under /docs/security/** (see .gitignore exceptions)
```

| Path | Purpose | Consumer |
|------|---------|----------|
| [audit-template.md](audit-template.md) | Template for `audit-vX.Y.Z.md` | `xtask security audit`, release |
| [release-checklist.md](release-checklist.md) | Pre-tag security checklist | Maintainers, rule 103 |
| [review-checklist.md](review-checklist.md) | Finding format for audits | Audit authors |
| [private-material-zero.md](private-material-zero.md) | PMZ policy | Scanner + CI + hooks |
| [waiver-requirements.md](waiver-requirements.md) | Waiver field requirements | Exception register |
| `audit-vX.Y.Z.md` | Per-version release audit | `scripts/verify-release-audit.sh`, `release.yml` |

Root policy entry: [`SECURITY.md`](../../SECURITY.md).

```text
PRODUCTION_SIGNING_EXECUTED = NO  (signing design lives under security/signing/)
PUBLICATION_STATUS = FORBIDDEN until release owner ceremony
```
