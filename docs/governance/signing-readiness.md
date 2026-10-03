# Signing governance readiness

```text
DOCUMENT_ROLE = SIGNING_GOVERNANCE_READINESS
PRODUCTION_SIGNING_EXECUTED = YES
PUBLIC_RELEASE = v0.4.4.1
PUBLICATION_STATUS = PUBLISHED_V0441
FINAL_PUBLISHED_RELEASE_HEAD = 99bd7c547cb4d7c7821a1e131dfca5cf8d8db987
AUTHORIZES_TAG = NO   # this document alone never authorizes a future tag
AUTHORIZES_GHCR = NO
AUTHORIZES_GITHUB_RELEASE = NO
```

This document productionizes the **design and verification path** for release
signing. Production signatures for private **`v0.4.3`** were executed under a
separate owner ceremony (SSH tag + Cosign blob + Cosign OCI). Future releases
still require explicit owner authorization.

Canonical public trust material and policy:

- [`security/signing/TRUST-POLICY.md`](../../security/signing/TRUST-POLICY.md)
- [`security/signing/README.md`](../../security/signing/README.md)
- Public keys under `security/signing/` (never private keys)

## Signature types (explicit)

| Surface | Type | Authenticated object | Public material |
|---------|------|----------------------|-----------------|
| Git tag | **SSH** annotated tag signing | Tag object → commit | `exyonq-release-tag-ssh-v1.pub`, `exyonq-release-tag-signers` |
| Artifacts / checksums | **Cosign blob** (`sign-blob`) | Root object `SHA256SUMS.txt` | `exyonq-cosign.pub` |
| OCI | **Cosign OCI** digest signing | Image digest `@sha256:…` only (mutable tags forbidden) | `exyonq-cosign.pub` |

```text
TAG_SIGNING_GOVERNANCE = READY
ARTIFACT_SIGNING_GOVERNANCE = READY
OCI_SIGNING_GOVERNANCE = READY
TRUST_ROOT_STATUS = READY_FOR_RELEASE
PUBLIC_TRUST_ROOT_PUBLISHED = YES   # public verify material on GitHub Release v0.4.3
V043_PRODUCTION_SIGNING = PASS
```

## Verification tooling (must exist; exercised without production keys)

| Script | Role |
|--------|------|
| `scripts/release/verify-tag-signature.sh` | SSH tag verify |
| `scripts/release/verify-checksums.sh` | Checksum integrity |
| `scripts/release/verify-cosign-blob.sh` | Cosign blob verify |
| `scripts/release/verify-oci-signature.sh` | OCI digest verify (+ dry-run preconditions) |
| `scripts/release/verify-release-bundle.sh` | Bundle orchestration |
| `scripts/release/tests/p14sign-phase3.sh` / `p14sign-phase5-proof.sh` | Ephemeral **TEST_ONLY** / **NON_PRODUCTION** fixtures |

Production release signing remains an **explicit manual owner ceremony**.
Cursor agents must not access private signing media (`EXYONQ-SIGNING-A` volume paths are forbidden in-repo).

## Readiness vs execution

| Check | Status |
|-------|--------|
| Design + public trust files present | YES |
| Verify scripts present | YES |
| Ephemeral selftests allowed | YES (marked non-production) |
| Production private keys in repo | NO (forbidden) |
| Production signing executed in this remediation | **NO** |

```text
SIGNATURE_VERIFICATION = PASS   # tooling + public trust path
PRODUCTION_SIGNING_EXECUTED = NO
```
