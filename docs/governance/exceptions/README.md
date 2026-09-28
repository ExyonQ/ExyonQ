# Exception / waiver register (minimal)

```text
REGISTER_ROLE = RELEASE_GOVERNANCE_EXCEPTIONS
SCOPE = release-relevant waivers only (not full GRC-P1 catalog)
ENFORCEMENT = scripts/gates/waiver-expiry-gate.sh
```

Live entries:

| EXCEPTION_ID | Status | Expiry/review |
|--------------|--------|---------------|
| [RUSTSEC-2026-0222](RUSTSEC-2026-0222.md) | ACTIVE_WAIVER | v0.4.4 |

Field requirements: [`docs/security/waiver-requirements.md`](../../security/waiver-requirements.md).
