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
V044_AUDIT = docs/security/audit-v0.4.4.md
V044_PUBLICATION_STATUS = TAG_PUSHED_WORKFLOW_BLOCKED
V044_GITHUB_RELEASE = NOT_PUBLISHED
V044_GHCR = NOT_PUBLISHED
V044_TAGGED_COMMIT_BLOCKED = 1ce0a7007171d4af6d1f9f4510137298783575ee
```

```text
PRODUCTION_SIGNING_EXECUTED = YES  (v0.4.3 private release; design under security/signing/)
PUBLICATION_STATUS = PRIVATE_V043_RELEASE_COMPLETE
PUBLIC_RELEASE = v0.4.3
RUSTSEC_2026_0222_EXCEPTION_STATUS = ACTIVE_WAIVER
VULNERABILITY_FIXED = NO
WAIVED_FOR_V043 = YES
EXPIRY_REVIEW = v0.4.4
```
