# ExyonQ release signing — public trust policy

```text
SIGNING_KEYS_STATUS = PROVISIONED
TRUST_ROOT_ACTIVE = YES_LOCAL_NOT_YET_PUBLISHED
OFFICIAL_RELEASE_AUTHENTICITY_AVAILABLE = NO
P14SIGN_PUBLIC_TRUST_POLICY = ACTIVE_LOCAL
PUBLIC_TRUST_ROOT_PUBLISHED = NO
PUBLICATION_STATUS = LOCAL_ACTIVATION_ONLY
OUT_OF_BAND_TRUST_ANCHOR = DESIGNED_NOT_IMPLEMENTED
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
TRUST_GENERATION = v1
ACTIVATED_AT = 2026-07-27T13:23:01Z
SSH_KEY_ID = EXYONQ_RELEASE_TAG_SSH_V1
COSIGN_KEY_ID = EXYONQ_RELEASE_COSIGN_V1
```

### Fingerprints (exact)

```text
SSH_FINGERPRINT =
  SHA256:U2Mnxtmcl1Xz7dw6lyMaLciwsBM/V/vmmDyU4iljz1c

COSIGN_PUBLIC_KEY_SHA256 =
  83931e3916b5d50b571fbc4926f7eef6ed1031b23f62ef17c1bf2780b6031562
```

## Surfaces

| Surface | Mechanism | Authenticated object |
|---------|-----------|----------------------|
| Git tag | SSH signing (annotated tags only) | Tag object → commit |
| Release bundle | Cosign key-managed `sign-blob` | **`SHA256SUMS.txt`** (root signed object) |
| OCI | Cosign key-managed | Image **digest only** (`@sha256:…`) — mutable tags forbidden |

`release-manifest.json` and artifacts are authenticated because their hashes appear in `SHA256SUMS.txt`. Signature files and Cosign bundles are **not** listed in `SHA256SUMS.txt`.

## Active keys

| Key ID | Role | Public material |
|--------|------|-----------------|
| `EXYONQ_RELEASE_TAG_SSH_V1` | Annotated release tags (`git tag -s -a`) | `exyonq-release-tag-ssh-v1.pub`, `exyonq-release-tag-signers` (principal `exyonq-release`) |
| `EXYONQ_RELEASE_COSIGN_V1` | Artifact blob **and** OCI digest (MODEL_1) | `exyonq-cosign.pub` |

## Key model (frozen)

```text
SSH_TAG_KEY = separate
COSIGN_BLOB_AND_OCI_KEY = shared
SUCCESSOR_KEY = distinct (generated on incident; not pre-created)
ROOT_SIGNED_OBJECT = SHA256SUMS.txt
OCI_SIGNING = DIGEST_ONLY
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
SYNC_TO_GITHUB_STAGING = NO
```
