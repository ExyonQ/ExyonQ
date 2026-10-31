# V044 post-WAF dependency baseline seal

```text
TRACK = V044_WAF_INTEGRATION
V044_DEPENDENCY_BASELINE_SEAL_STATUS = SUPERSEDED_BY_POST_WAF_SEAL
V044_POST_WAF_DEPENDENCY_BASELINE_SEAL = PASS
```

## Authority

Owner-authorized product expansion of native WAF into the v0.4.4 integration line.
Historical D01–D11 evidence remains valid for their recorded trees; this seal is the
dependency baseline for subsequent capabilities after WAF integration.

## Pre-integration baseline (recorded)

```text
WAFINT_HEAD_BEFORE = 255df1f9d310457e659d17e0a9030f2fb1c405c5
WAFINT_TREE_BEFORE = ac35df0737f7dd264954edce250f7534e734fd8a
WAFINT_CARGO_LOCK_SHA256_BEFORE = 0d5c78445354fdd3319a3af2ed4b152456f33be0ec631fd99a030fc8c37c930e
WAFINT_RUST_TOOLCHAIN_BEFORE = 1.94.0
```

## Intentional dependency delta

```text
DEPENDENCY_BASELINE_DRIFT = YES_AUTHORIZED_WAF_RECONCILIATION
NEW_DIRECT_WORKSPACE_PACKAGES = exyonq-waf-api, exyonq-waf
NEW_EXTERNAL_LOCK_PACKAGE = ipnetwork 0.20.0
ALREADY_PRESENT_REUSED = regex, hmac, sha2, getrandom, arc-swap, thiserror
REMOVED_DEPENDENCIES = (none)
VERSION_CHANGES = (none outside new packages)
```

Boundary validation:

```text
CORE_DEPENDS_ON_WAF_API = YES
CORE_DEPENDS_ON_CONCRETE_WAF = NO
CLI_DEPENDS_ON_CONCRETE_WAF = YES
```

## Verification (integration track — not Cap015)

| Check | Result |
| --- | --- |
| `cargo audit` (post-lock) | 0 vulnerabilities |
| `project-integrity-gate --tree .` | PASS |
| Darwin `cargo check` WAF/core/CLI | PASS |
| Netcup amd64 `cargo check` WAF/core/CLI | PASS |
| Oracle arm64 `cargo check` WAF/core/CLI | PASS |
| Netcup amd64 real serve WAF block XSS | PASS (200 benign / 403 block) |
| Oracle arm64 real serve WAF block XSS | PASS (200 benign / 403 block) |

```text
CAPABILITY_015_STARTED = NO
CAP015_WAF_IMPLEMENTATION_STATUS = READY_FOR_REAL_PRODUCTION_QUALIFICATION
```

## Seal fields

```text
POST_WAF_SEALED_HEAD = 48db7f78d480c0265dad3564babd36a5c7b9f68a
POST_WAF_SEALED_TREE = 6767ff8585a323f0ff525e8cdac85fe55b7b0fa7
POST_WAF_CARGO_LOCK_SHA256 = 82f6e23724995ba7ec413b36732fc6c20694a1c73be5cd629176852cd522685a
POST_WAF_RUST_TOOLCHAIN = 1.94.0
```

Note: committing this filled seal advances HEAD; the sealed product/lock identity
above binds the four integration commits through `48db7f78…` (docs close). A
follow-up seal-fill commit may differ in HEAD/TREE but must keep the same
`POST_WAF_CARGO_LOCK_SHA256` unless dependencies change again.
