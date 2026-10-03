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
V0441_AUDIT = docs/security/audit-v0.4.4.1.md
V0441_PUBLICATION_STATUS = PUBLISHED
V0441_GITHUB_RELEASE = https://github.com/ExyonQ/ExyonQ/releases/tag/v0.4.4.1
V0441_GHCR = ghcr.io/exyonq/exyonq:0.4.4.1
V044_AUDIT = docs/security/audit-v0.4.4.md
V044_PUBLICATION_STATUS = PUBLISHED
V044_GITHUB_RELEASE = https://github.com/ExyonQ/ExyonQ/releases/tag/v0.4.4
V044_GHCR = ghcr.io/exyonq/exyonq:0.4.4
V044_TAGGED_COMMIT = 8f3336f783dc546bdaad4a60b5501568bf688b3a
```

```text
PRODUCTION_SIGNING_EXECUTED = YES  (v0.4.3 private release; design under security/signing/)
PUBLICATION_STATUS = PUBLISHED_V0441
PUBLIC_RELEASE = v0.4.4.1
RUSTSEC_2026_0222_EXCEPTION_STATUS = ACTIVE_WAIVER
VULNERABILITY_FIXED = NO
WAIVED_FOR_V043 = YES
EXPIRY_REVIEW = v0.4.4
```
