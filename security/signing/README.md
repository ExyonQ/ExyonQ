# ExyonQ release signing (public)

```text
SIGNING_KEYS_STATUS = PROVISIONED
TRUST_ROOT_ACTIVE = YES_LOCAL_NOT_YET_PUBLISHED
OFFICIAL_RELEASE_AUTHENTICITY_AVAILABLE = NO
P14SIGN_PUBLIC_TRUST_POLICY = ACTIVE_LOCAL
PUBLICATION_STATUS = LOCAL_ACTIVATION_ONLY
PUBLIC_TRUST_ROOT_PUBLISHED = NO
ARTIFACT_SIGNATURE_AUTHORITATIVE = SSH_ED25519_V2_OVER_SHA256SUMS
```

This directory holds **public** verification material only — never private keys.

```text
TLS_TEST_FIXTURES_ARE_RELEASE_SIGNING_KEYS = NO
PRIVATE_KEY_IN_GIT = NO
DO_NOT_TREAT_GITHUB_AS_SOLE_TRUST_ROOT = YES
```

| Path | Role |
| --- | --- |
| `TRUST-POLICY.md` | Active local trust policy |
| `fingerprints.txt` | Exact SSH + Cosign fingerprints |
| `exyonq-release-tag-signers` | SSH `allowedSignersFile` (principal `exyonq-release`) |
| `exyonq-release-artifact-signers` | SSH artifact `allowedSignersFile` (principal `exyonq-release`, namespace `file`) |
| `exyonq-release-tag-ssh-v2.pub` | Active local SSH public key (v2) |
| `exyonq-release-tag-ssh-v1.pub` | Superseded local SSH public key (v1; historical tags only) |
| `exyonq-cosign.pub` | Official Cosign public key (blob + OCI, MODEL_1) |

## Inspect fingerprints

```bash
ssh-keygen -lf security/signing/exyonq-release-tag-ssh-v2.pub

shasum -a 256 security/signing/exyonq-cosign.pub
```

Expected:

```text
SSH_FINGERPRINT =
  SHA256:7sMmeEgNFoOZa3xvaMgJPs4T724wWxiWMaFfW3Iusfs

SUPERSEDED_SSH_FINGERPRINT =
  SHA256:U2Mnxtmcl1Xz7dw6lyMaLciwsBM/V/vmmDyU4iljz1c

COSIGN_PUBLIC_KEY_SHA256 =
  83931e3916b5d50b571fbc4926f7eef6ed1031b23f62ef17c1bf2780b6031562
```

## Verify a future release tag (not created yet)

`v0.4.0` **does not exist yet**. The command below is the future operator instruction after an authorized signed tag exists:

```bash
git -c gpg.format=ssh \
  -c gpg.ssh.allowedSignersFile=security/signing/exyonq-release-tag-signers \
  verify-tag v0.4.0
```

Or via the harness (when a signed `v0.4.0` and matching commit exist):

```bash
bash scripts/release/verify-tag-signature.sh \
  --repo . \
  --tag v0.4.0 \
  --expected-version 0.4.0 \
  --expected-commit <forty-char-lowercase-commit> \
  --allowed-signers security/signing/exyonq-release-tag-signers
```

## Verify a future release bundle Cosign blob

```bash
cosign verify-blob \
  --key security/signing/exyonq-cosign.pub \
  --signature SHA256SUMS.txt.sig \
  --bundle SHA256SUMS.txt.bundle \
  SHA256SUMS.txt
```

## Verify an SSH artifact signature

The artifact root is `SHA256SUMS.txt`; the SSH signature uses namespace `file`
and the principal `exyonq-release`. This is the authoritative artifact layer
for the local candidate. Official releases additionally require the Cosign
layer above.

```bash
bash scripts/release/verify-release-bundle.sh \
  --bundle-root <RELEASE_BUNDLE_DIR> \
  --verify-artifact-signature-ssh \
  --ssh-allowed-signers security/signing/exyonq-release-artifact-signers
```

Build and sign a local candidate without exposing the key:

```bash
bash scripts/release/build-signed-release-bundle.sh \
  --signing-key "$EXYONQ_RELEASE_SSH_KEY" \
  --out-dir .exyonq-local/signing/local-candidate
```

The output is always `LOCAL_CANDIDATE_NOT_FOR_RELEASE`; it never publishes,
tags, pushes or deploys. The private key must remain owner-only and outside the
repository and the bundle.

Offline / controlled environments may also use the harness:

```bash
bash scripts/release/verify-cosign-blob.sh \
  --checksums SHA256SUMS.txt \
  --key security/signing/exyonq-cosign.pub \
  --signature SHA256SUMS.txt.sig \
  --bundle SHA256SUMS.txt.bundle
```

## Verify a future OCI image (digest only)

```bash
cosign verify \
  --key security/signing/exyonq-cosign.pub \
  ghcr.io/exyonq/exyonq@sha256:<digest>
```

Mutable tags (`:latest`, unpinned tags) are forbidden for official authenticity.

## Integrity-only bundle check (no authenticity claim)

```bash
bash scripts/release/verify-release-bundle.sh \
  --bundle-root <RELEASE_BUNDLE_DIR> \
  --integrity-only
```

`--integrity-only` never claims authenticity.
