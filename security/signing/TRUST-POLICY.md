# ExyonQ release signing — public trust policy

```text
SIGNING_KEYS_STATUS = PROVISIONED
TRUST_ROOT_ACTIVE = YES_LOCAL_NOT_YET_PUBLISHED
OFFICIAL_RELEASE_AUTHENTICITY_AVAILABLE = NO
P14SIGN_PUBLIC_TRUST_POLICY = ACTIVE_LOCAL
PUBLIC_TRUST_ROOT_PUBLISHED = NO
PUBLICATION_STATUS = LOCAL_ACTIVATION_ONLY
OUT_OF_BAND_TRUST_ANCHOR = DESIGNED_NOT_IMPLEMENTED
ARTIFACT_SIGNATURE_AUTHORITATIVE = SSH_ED25519_V2_OVER_SHA256SUMS
ARTIFACT_SIGNATURE_NAMESPACE = file
COSIGN_ARTIFACT_LAYER = RETAINED_REQUIRED_FOR_OFFICIAL_RELEASE
LOCAL_ARTIFACT_CANDIDATE = SSH_ONLY_NOT_OFFICIAL
```

```text
OFFICIAL_KEYS_PROVISIONED = YES
DO_NOT_TREAT_GITHUB_AS_SOLE_TRUST_ROOT = YES
TLS_TEST_FIXTURES_ARE_RELEASE_SIGNING_KEYS = NO
PRIVATE_KEY_IN_GIT = NO
V0_4_0_SIGNED = NO
OFFICIAL_RELEASE_AVAILABLE = NO
```

This trust root is **active for local verification inside the development tree**.
Public keys have **not** been synchronized to GitHub staging or published remotely.

## Activation

```text
TRUST_GENERATION = v2
ACTIVATED_AT = 2026-11-05T19:55:33Z
SSH_KEY_ID = EXYONQ_RELEASE_TAG_SSH_V2
SUPERSEDED_SSH_KEY_ID = EXYONQ_RELEASE_TAG_SSH_V1
COSIGN_KEY_ID = EXYONQ_RELEASE_COSIGN_V1
```

### Fingerprints (exact)

```text
SSH_FINGERPRINT =
  SHA256:7sMmeEgNFoOZa3xvaMgJPs4T724wWxiWMaFfW3Iusfs

SUPERSEDED_SSH_FINGERPRINT =
  SHA256:U2Mnxtmcl1Xz7dw6lyMaLciwsBM/V/vmmDyU4iljz1c

COSIGN_PUBLIC_KEY_SHA256 =
  83931e3916b5d50b571fbc4926f7eef6ed1031b23f62ef17c1bf2780b6031562
```

## SSH key rotation v1 -> v2

On 2026-11-05 the SSH tag key was rotated from `EXYONQ_RELEASE_TAG_SSH_V1`
to `EXYONQ_RELEASE_TAG_SSH_V2`. V2 is active for new annotated release tags.
V1 is `SUPERSEDED`; it remains in `exyonq-release-tag-signers` only so tags
signed before the rotation stay verifiable. This is a local activation: the
public trust root has not been published and no tag or release is authorized
by this document.

## Surfaces

| Surface | Mechanism | Authenticated object |
| --- | --- | --- |
| Git tag | SSH signing (annotated tags only) | Tag object → commit |
| Release bundle (local candidate) | SSH V2, namespace `file` | **`SHA256SUMS.txt`** (root signed object) |
| Release bundle (official) | SSH V2 **and** Cosign key-managed `sign-blob` | **`SHA256SUMS.txt`** (both layers required) |
| OCI | Cosign key-managed | Image **digest only** (`@sha256:…`) — mutable tags forbidden |

`release-manifest.json` and artifacts are authenticated because their hashes appear in `SHA256SUMS.txt`. Signature files and Cosign bundles are **not** listed in `SHA256SUMS.txt`.

The SSH artifact signature is authoritative for the release bundle. Cosign is
retained as a second, independent layer and is required before an **official**
release. A local candidate is explicitly `LOCAL_CANDIDATE_NOT_FOR_RELEASE`; it
does not claim the official Cosign layer and must not be published as a release.

## Active keys

| Key ID | Role | Public material |
| --- | --- | --- |
| `EXYONQ_RELEASE_TAG_SSH_V2` | Active annotated release tags (`git tag -s -a`) and artifact root signatures (namespace `file`) | `exyonq-release-tag-ssh-v2.pub`, `exyonq-release-tag-signers`, `exyonq-release-artifact-signers` (principal `exyonq-release`) |
| `EXYONQ_RELEASE_TAG_SSH_V1` | Superseded; historical tags only | `exyonq-release-tag-ssh-v1.pub` |
| `EXYONQ_RELEASE_COSIGN_V1` | Artifact blob **and** OCI digest (MODEL_1) | `exyonq-cosign.pub` |

## Key model (frozen)

```text
SSH_RELEASE_KEY = tag_and_artifact_root
COSIGN_BLOB_AND_OCI_KEY = shared
SUCCESSOR_KEY = distinct (generated on incident; not pre-created)
ROOT_SIGNED_OBJECT = SHA256SUMS.txt
OCI_SIGNING = DIGEST_ONLY
```

## Artifact signature path

The local, reproducible entrypoint is:

```bash
bash scripts/release/build-signed-release-bundle.sh \
  --signing-key "$EXYONQ_RELEASE_SSH_KEY" \
  --out-dir .exyonq-local/signing/local-candidate
```

The key must be owner-only, outside the repository bundle, and its public
fingerprint must match `SSH_FINGERPRINT` in `fingerprints.txt`. The script signs
`SHA256SUMS.txt` with `ssh-keygen -Y sign -n file`, verifies it against the
versioned `exyonq-release-artifact-signers`, verifies every checksum, and scans
the bundle for private material. It prints `OFFICIAL_RELEASE=NO`; it never
publishes, tags, pushes or deploys.

Verification without private material:

```bash
bash scripts/release/verify-release-bundle.sh \
  --bundle-root <bundle> \
  --verify-artifact-signature-ssh \
  --ssh-allowed-signers security/signing/exyonq-release-artifact-signers
```

## Rotation / revocation states

```text
ACTIVE
SUPERSEDED
RETIRED
EXPIRED
REVOKED
```

```text
ORDINARY_ROTATION = SUPERSEDED | RETIRED | EXPIRED
  NORMAL_ROTATION_TRANSITION =
    SIGNED_BY_OLD_VALID_KEY_AND_NEW_KEY_WHEN_AVAILABLE

COMPROMISED_KEY = REVOKED
  COMPROMISE_TRANSITION =
    OUT_OF_BAND_EMERGENCY_NOTICE_PLUS_NEW_KEY_FINGERPRINT
  COMPROMISE_RECOVERY =
    ACTIVATE_DISTINCT_SUCCESSOR_WITH_PUBLISHED_TRANSITION
```

Ordinary rotation does **not** use `REVOKED`. A transition signed only by a compromised key is insufficient.

## Distribution vs trust

GitHub (and future staging sync) is a **distribution channel**, not the sole root of trust.
Consumers should pin fingerprints from `fingerprints.txt` and prefer an out-of-band confirmation when available.

```text
P14SIGN_OUT_OF_BAND_TRUST_ANCHOR_DESIGNED = YES
P14SIGN_OUT_OF_BAND_TRUST_ANCHOR_IMPLEMENTED = NO
```

## What this does **not** claim

```text
PUBLIC_TRUST_ROOT_PUBLISHED = NO
OFFICIAL_RELEASE_AVAILABLE = NO
V0_4_0_SIGNED = NO
OFFICIAL_ARTIFACT_COSIGN_SIGNATURE = NOT_EXECUTED
SYNC_TO_GITHUB_STAGING = NO
```
