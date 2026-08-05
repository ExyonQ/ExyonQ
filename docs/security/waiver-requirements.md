# Release security waiver requirements

```text
POLICY_ID = EXYONQ-RELEASE-WAIVER-REQUIREMENTS
STATUS = MANDATORY
REGISTER = docs/governance/exceptions/
FAIL_MODE = FAIL_CLOSED
```

Medium+ findings that are not fixed before a tag require a tracked exception
record under `docs/governance/exceptions/` with at least:

| Field | Required |
|-------|----------|
| EXCEPTION_ID | YES |
| source finding/advisory | YES |
| scope | YES |
| affected version | YES |
| reason | YES |
| risk/severity | YES |
| reachability rationale | YES |
| approver/owner decision | YES |
| compensating controls | YES |
| created date | YES |
| expiry/revisit trigger | YES |
| target version for remediation | YES |
| evidence | YES |
| status | YES |

```text
Waivers without expiry/revisit = INVALID
Expired waivers = RELEASE BLOCKED
Wording must not imply a vulnerability is fixed when it is only waived
```

Canonical live register entries are markdown files named `EXQ-EXC-*.md` (or
advisory-scoped IDs such as `RUSTSEC-2026-0222.md`).
