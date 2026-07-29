# Security Policy

ExyonQ is currently a **private maturation** project. This policy describes
supported release lines and how to report security issues without public
disclosure of sensitive details.

## Supported versions

Only the current private tip receives security fixes under this policy:

| Version | Supported |
|---------|-----------|
| 0.4.2   | Yes       |
| 0.4.1   | No        |
| 0.4.0   | No        |
| < 0.4.0 | No        |

Any release line older than `0.4.0` is **not** supported for security fixes.

## Project status

- The GitHub repository and GHCR package are **private**.
- Public opening is **not** authorized by this policy.
- The mutable container tag `latest` is **not** part of the supported release
  contract for security response.

## Reporting a vulnerability

Send undisclosed or sensitive security reports **exclusively** by email to:

**security@exyonq.org**

Recommended subject line:

```text
[SECURITY] ExyonQ vulnerability report
```

**Do not open public GitHub Issues** for undisclosed vulnerabilities, or for
reports that include exploit details, credentials, or private material.

GitHub Private Vulnerability Reporting is **not** claimed as available for this
repository at this time. If it is enabled after a future public opening, this
policy may be updated to list it as a primary or complementary channel without
necessarily removing email reporting.

### What to include (when possible)

- ExyonQ version (for example `0.4.2`)
- Git commit SHA and/or `source_revision` from `exyonq --version`
- Operating system and architecture
- Relevant configuration (sanitized; no secrets)
- Impact description
- Minimal reproduction steps
- A safe proof of concept (no destructive payloads against third parties)
- Logs with credentials and personal data removed
- Whether the issue has already been disclosed elsewhere

### Do not include or attach

- Private keys or PEM/OpenSSH private blocks
- Passphrases
- Tokens
- Passwords
- Cookies or session material
- Personal data unrelated to the defect
- Infrastructure credentials
- Third-party material without authorization

This aligns with repository policy **EXYONQ-SEC-PRIVATE-MATERIAL-ZERO**:
private-key-formatted material must not be committed, packaged, or published,
including in security reports and fixtures.

## Acknowledgement

We will acknowledge valid security reports as soon as reasonably possible and
coordinate remediation and disclosure directly with the reporter.

## Scope (informative)

In scope when evaluating reports: ExyonQ HTTP parsing and serving paths,
static file resolution, reverse proxy, config reload, TLS termination,
HTTP/3 QUIC (when enabled), control socket (`exyonqctl`), and official modules
(metrics, compression, ratelimit, WASM host surfaces that ship with the
product).

Out of scope: third-party benchmark containers, rival server configurations,
and compatibility importers except when they emit unsafe native config.

## Release security gate (maintainers)

Each tag `vX.Y.Z` is expected to satisfy the project’s security release
checklist and related CI security workflows before publication. Historical
audit documents under older version schemes are not current supported-version
claims and are not linked from this live policy.
