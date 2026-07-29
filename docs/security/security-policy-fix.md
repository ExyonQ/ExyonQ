# P14SECURITYPOLICY-FIX — Security Policy Correctness Fix

```text
DOCUMENT = docs/security/security-policy-fix.md
TRACK = P14SECURITYPOLICY-FIX
KIND = DOCUMENTARY_CORRECTNESS_FIX

P14SECURITYPOLICY_FIX_STATUS = READY_FOR_OWNER_DIFF_REVIEW

P14V042_REOPENED = NO
P14V043_OPENED = NO
P14V043_SCOPE_PREPARATION = PAUSED

PUSH = NO
TAG = NO
RELEASE = NO
GHCR_PUSH = NO
SIGNING = NO
LATEST_CHANGED = NO
VISIBILITY_CHANGED = NO
```

## Outcomes

```text
SECURITY_POLICY_SOURCE_FILE = SECURITY.md
SECURITY_POLICY_OLD_SUPPORTED_VERSION = 0.3.x
SECURITY_POLICY_NEW_SUPPORTED_VERSION = 0.4.2
SECURITY_POLICY_REPORTING_CHANNEL = security@exyonq.org
SECURITY_POLICY_PRIVATE_REPORTING = EMAIL
SECURITY_POLICY_PUBLIC_ISSUES_FOR_UNDISCLOSED_VULNS = FORBIDDEN
SECURITY_POLICY_BROKEN_AUDIT_LINK_REMOVED = YES
SECURITY_POLICY_GITHUB_PVR_CLAIMED = NO

SECURITY_POLICY_VERSION_TABLE = PASS
SECURITY_POLICY_BROKEN_LINKS = 0
SECURITY_POLICY_STALE_LIVE_REFERENCES = 0
SECURITY_POLICY_PRIVATE_MATERIAL_GUIDANCE = PASS
SECURITY_POLICY_HISTORICAL_LEDGERS_UNCHANGED = YES
COMMIT_PREPARED = YES
```

## Owner channel decision

```text
SECURITY_POLICY_REPORTING_CHANNEL = security@exyonq.org
SECURITY_EMAIL_AUTHORIZED_FOR_VULNERABILITY_REPORTS = YES
GITHUB_PRIVATE_VULNERABILITY_REPORTING = NOT_CURRENTLY_AVAILABLE
```

## Defect corrected

GitHub Security Policy is fed by repository-root `SECURITY.md` (not
`.github/SECURITY.md`).

Stale live content claimed an obsolete supported-version line (see
`SECURITY_POLICY_OLD_SUPPORTED_VERSION`) and linked a historical audit path
under `docs/security/` that is not published on GitHub (`/docs/` is
gitignored; the target was not force-tracked). That produced an obsolete
version impression on the Security policy page.

## Contact coherence

| Location | Address | Role |
|---|---|---|
| `SECURITY.md` | `security@exyonq.org` | Official vulnerability reporting |
| `packaging/nfpm.yaml` | `security@exyonq.org` | Package maintainer contact (unchanged) |

No automatic rewrite of unrelated maintainer fields. No contradiction found.

## Files

- `SECURITY.md` — live policy rewrite
- `docs/security/security-policy-fix.md` — this ledger (`git add -f`)

Historical `docs/releases/p14v04*` ledgers: **unchanged**.

## Commit

```text
docs(security): correct supported versions and reporting policy
```

Prepared locally on branch `docs/p14securitypolicy-fix`. **No push** until
express owner authorization.
