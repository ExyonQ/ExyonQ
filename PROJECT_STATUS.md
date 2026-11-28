# ExyonQ — Project Status (canonical)

```text
DOCUMENT = PROJECT_STATUS.md
DOCUMENT_ROLE = CANONICAL_PROJECT_STATUS
AUTHORITY = SOLE for the current release identity and public-release state
```

<!-- BEGIN RELEASE_STATUS (generated from RELEASE_STATUS; do not edit) -->
VERSION = 0.9.3
PUBLICATION_STATUS = PUBLISHED
PREVIOUS_PUBLIC_VERSION = 0.9.2
PUBLIC_RELEASE = v0.9.2
RELEASE_NOTE = docs/release/v0.9.3.md
AUDIT_NOTE = docs/security/audit-v0.9.3.md
<!-- END RELEASE_STATUS -->

## Current state

- The workspace version is **0.9.3** and is **not published**: no tag, GitHub
  Release, image or cosign signature exists for it.
- The latest public release is **v0.9.2**. Its binaries, GHCR images and Docker
  Hub mirror were verified by digest; see `docs/release/v0.9.2.md`.
- The 0.9.3 candidate was merged into `main` as `1b92e6a` (PR #110) and closes
  issue #109. Publication still requires the owner's explicit authorization.
- Release identity is generated from `RELEASE_STATUS`; do not edit the block
  above. `scripts/gates/readme-release-version-gate.sh --tree .` fails closed on
  a stale block, a workspace-version mismatch or a contradictory public-release
  claim.

## Sources

| Subject | Document |
| --- | --- |
| Running history | [`CHANGELOG.md`](CHANGELOG.md) |
| Release notes | [`docs/release/`](docs/release/) |
| Security policy and audits | [`SECURITY.md`](SECURITY.md), [`docs/security/`](docs/security/) |
| Global roadmap | [`ROADMAP.md`](ROADMAP.md) |
| Documentation index | [`DOCUMENTATION_INDEX.md`](DOCUMENTATION_INDEX.md) |

Historical lab and pre-0.9 release material remains in `CHANGELOG.md` and the
tracked release/governance records; it is not evidence for the current release
identity.
