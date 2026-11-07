# Signing governance readiness

```text
DOCUMENT_ROLE = SIGNING_GOVERNANCE_READINESS
PRODUCTION_SIGNING_EXECUTED = YES
PUBLIC_RELEASE = v0.4.3
PUBLICATION_STATUS = PRIVATE_V043_RELEASE_COMPLETE
FINAL_PUBLISHED_RELEASE_HEAD = 3a8af75dd0939e5aae2f4d09b842573499e15a1c
AUTHORIZES_TAG = NO   # this document alone never authorizes a future tag
AUTHORIZES_GHCR = NO
AUTHORIZES_GITHUB_RELEASE = NO
```

## Current 0.9.1 local artifact path

```text
CURRENT_TRUST_POLICY = LOCAL_ACTIVATION_ONLY
CURRENT_PUBLIC_TRUST_ROOT_PUBLISHED = NO
CURRENT_ARTIFACT_SIGNATURE = SSH_ED25519_V2_OVER_SHA256SUMS
CURRENT_LOCAL_CANDIDATE = LOCAL_CANDIDATE_NOT_FOR_RELEASE
CURRENT_OFFICIAL_COSIGN_SIGNATURE = NOT_EXECUTED
```

The current artifact path signs the root `SHA256SUMS.txt` with the V2 ED25519
key (namespace `file`) and verifies it against the versioned
`security/signing/exyonq-release-artifact-signers`. Cosign remains a required
second layer for an official release; the local candidate does not claim it.

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
| --- | --- | --- | --- |
| Git tag | **SSH** annotated tag signing | Tag object → commit | `exyonq-release-tag-ssh-v2.pub`, `exyonq-release-tag-signers` (v1 remains historical) |
| Artifacts / checksums | **SSH V2** root signature; Cosign for official releases | Root object `SHA256SUMS.txt` | `exyonq-release-artifact-signers`, `exyonq-release-tag-ssh-v2.pub`; `exyonq-cosign.pub` |
| OCI | **Cosign OCI** digest signing | Image digest `@sha256:…` only (mutable tags forbidden) | `exyonq-cosign.pub` |

```text
TAG_SIGNING_GOVERNANCE = READY
ARTIFACT_SIGNING_GOVERNANCE = READY
OCI_SIGNING_GOVERNANCE = READY
TRUST_ROOT_STATUS = READY_FOR_RELEASE
PUBLIC_TRUST_ROOT_PUBLISHED = YES   # historical v0.4.3 only; current 0.9.1 state is NO above
V043_PRODUCTION_SIGNING = PASS
```

## Verification tooling (must exist; exercised without production keys)

| Script | Role |
| --- | --- |
| `scripts/release/verify-tag-signature.sh` | SSH tag verify |
| `scripts/release/verify-checksums.sh` | Checksum integrity |
| `scripts/release/verify-ssh-signature.sh` | SSH artifact root verify |
| `scripts/release/verify-cosign-blob.sh` | Cosign blob verify |
| `scripts/release/verify-oci-signature.sh` | OCI digest verify (+ dry-run preconditions) |
| `scripts/release/verify-release-bundle.sh` | Bundle orchestration |
| `scripts/release/build-signed-release-bundle.sh` | Local build + scan + SSH sign + verify |
| `scripts/release/tests/p14sign-phase3.sh` / `p14sign-phase5-proof.sh` | Ephemeral **TEST_ONLY** / **NON_PRODUCTION** fixtures |

Production release signing remains an **explicit manual owner ceremony**.
Cursor agents must not access private signing media (`EXYONQ-SIGNING-A` volume paths are forbidden in-repo).

## Readiness vs execution

| Check | Status |
| --- | --- |
| Design + public trust files present | YES |
| Verify scripts present | YES |
| Ephemeral selftests allowed | YES (marked non-production) |
| Production private keys in repo | NO (forbidden) |
| Production signing executed in GRC-R5F remediation (historical) | **NO** |
| Production signing executed for private v0.4.3 (CURRENT) | **YES** |

```text
SIGNATURE_VERIFICATION = PASS   # tooling + public trust path + v0.4.3 ceremony
PRODUCTION_SIGNING_EXECUTED = YES
SSH_TAG_SIGNATURE_STATUS = PASS
COSIGN_BLOB_SIGNATURE_STATUS = PASS
COSIGN_OCI_SIGNATURE_STATUS = PASS
```
