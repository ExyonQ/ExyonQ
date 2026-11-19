# COMPAT matrix — ExyonQ 0.9.2, measured from the published release binaries

**Status:** measured 2026-11-19 from the assets of the published GitHub Release
`v0.9.2`; `RESULT=PASS COMPAT_MATRICES_GATE` on macOS arm64, Linux arm64 and
Linux amd64.
**Scope:** the three offline importers (NGINX, Apache httpd, OpenLiteSpeed
throttle) and the config compatibility promise. It measures what the shipped
binaries map; it is not a claim of total substitution — see
[compatibility-scope-v1.md](compatibility-scope-v1.md).

## Provenance

Every number below comes from binaries unpacked from the release assets listed
here, not from a local `target/` build and not from CI artifacts.

| Platform | Runtime | Release assets used | `exyonq --version` |
| --- | --- | --- | --- |
| macOS arm64 | native, this checkout's host | `exyonq-macos-arm64.tar.gz` `6145ec65f7bb008372952f47e64b52096044be7b00d8d531f9b85fad98144215` (ships `exyonq`, `exyonqctl`, `exyonq-compat`) | `source_tag=v0.9.2`, `source_revision=4e3338a…`, `target=aarch64-apple-darwin` |
| Linux arm64 | native, Ubuntu 24.04 aarch64 | `exyonq-0.9.2-linux-arm64.tar.gz` `c0fda1510e6ff2b724c8adf0da280987125e71f8ea406ac6ca79f7d315b9f301` (`exyonq`, `exyonqctl`) + `exyonq_0.9.2_arm64.deb` `e96fe3335973a54ea83c6e7215472d6896bf1c2ac71cfaa879321e528a0a89e5` (`exyonq-compat`) | `source_tag=v0.9.2`, `source_revision=4e3338a…`, `target=aarch64-unknown-linux-gnu` |
| Linux amd64 | native x86_64, Ubuntu 24.04; the binaries execute inside `cgr.dev/chainguard/wolfi-base` | `exyonq-0.9.2-linux-amd64.tar.gz` `d4865b3ed336f11b68bc4d688ffde619903b82d4dbe2c20e0174f0ed0f933b11` (`exyonq`, `exyonqctl`) + `exyonq_0.9.2_amd64.deb` `b287c0a0ee97027525f3c4292e2ece3ce0323f1d45302f13ecc5e1665e465807` (`exyonq-compat`) | `source_tag=v0.9.2`, `source_revision=4e3338a…`, `target=x86_64-unknown-linux-gnu` |

Two details worth stating instead of hiding:

- The WS6 product layout for Linux (`usr/` tree) ships the server and `exyonqctl`
  only; the compat CLI travels in the legacy flat tarballs and in the `.deb` /
  `.rpm`. The macOS tarball is flat and carries all three.
- The 0.9.2 amd64 binaries need **glibc 2.42** and the amd64 host has 2.39, so
  that run happened inside a Wolfi container with the release binaries mounted.
  This is the documented glibc limitation of the 0.9.2 amd64 artifacts, not a
  workaround for a failing measurement.

Inputs: `tests/fixtures/compat-matrix/` and `tests/fixtures/migrations/` as
tracked at `v0.9.2` (`git diff 4e3338a -- …` is empty; combined fingerprint of
both directories `4442f45d84590252df6d0fa7c7b278945e9db3edc4b0b580710f27f181214f63`).

## Commands

```bash
# macOS arm64, release tarball binaries
EXYONQCTL_BIN=<extract>/exyonqctl EXYONQ_COMPAT_BIN=<extract>/exyonq-compat \
  bash scripts/gates/compat-matrices-gate.sh

# Linux arm64 / amd64, release tarball + .deb binaries
EXYONQCTL_BIN=<extract>/usr/bin/exyonqctl EXYONQ_COMPAT_BIN=<extract>/usr/bin/exyonq-compat \
  bash scripts/gates/compat-matrices-gate.sh

# config compatibility promise, release binary
EXYONQ_BIN=<extract>/exyonq bash scripts/gates/config-compat-promise-gate.sh
```

## Matrix

Measured on all three platforms; the gate fails when a measurement drifts from
`tests/fixtures/compat-matrix/expectations.txt`, so equality below is enforced,
not asserted.

| Source | exact | approximated | unsupported | corpus size | P1 open | P2 open |
| --- | --- | --- | --- | --- | --- | --- |
| NGINX (5 real configs: WordPress, Laravel, SPA, Home Assistant, n8n) | 79 | 13 | 5 | 21 distinct directives | 0 | 0 |
| Apache httpd (`apache.conf`) | 0 | 1 | 0 | 17 directive instances | 0 | 0 |
| OpenLiteSpeed (`perClientConnLimit` → `[modules.ratelimit.client]`) | 0 | 0 | 0 | 9 emitted fields | 0 | 0 |

Gate output on each platform:

```text
COMPAT_MATRICES_GAPS P1_OPEN=0 P2_OPEN=0
MATRICES=3 P1_OPEN=0 P2_OPEN=0
RESULT=PASS COMPAT_MATRICES_GATE
```

Config compatibility promise, with the released `exyonq`:

```text
PASS: scripts/gates/fixtures/config-compat/v0.9.1-config.toml
PASS: an unknown key is rejected (exit 1)
RESULT=PASS previously published configs still validate
```

## Contrast against the published compatibility claims

| Claim (source) | Verdict here |
| --- | --- |
| "Las tres matrices COMPAT sin ❌ P1/P2 abiertos" (roadmap DoD item 1) | **Confirmed from the release binaries** on three platforms, `P1_OPEN=0 P2_OPEN=0`. |
| `COMPAT_NGINX.md` block: `exact=79 approximated=13 unsupported=5 directives=21 p1_open=0 p2_open=0` | **Confirmed**: the released `exyonqctl` measures exactly those numbers. |
| Apache "equivalent map" (static + redirect + TLS + proxy + PHP-FPM) with no total-substitution claim | **Consistent**: 17 directive instances, 1 approximated, 0 refused, 0 exact-promoted; refusals stay in `gaps.txt`. |
| OpenLiteSpeed "one `perClientConnLimit` block, not a full OLS importer" | **Consistent**: one block, 9 emitted fields, no unsupported entries claimed. |
| `TOTAL_COMPAT_PROMISE` **FORBIDDEN** for Apache and NGINX until their gate corpus passes on Netcup AMD64 and Oracle ARM64 | **Honoured**: this matrix is the corpus measurement on both native hosts; it does not upgrade the promise to total substitution. |
| `config-compatibility-promise-v1.md` promise 1: a config that validated on a published 0.9.x release keeps validating | **Confirmed**: the literal `v0.9.1` config validates with the released 0.9.2 binary, and the negative control (unknown key) is rejected. |

## What this matrix does not show

- It does not run the imported configurations on a live server; that is covered
  by the E2E scripts and by `exyonq validate` in the config gates, not here.
- macOS is measured on arm64 only; Windows is not a supported target.
- The amd64 run needed a Wolfi container because of the glibc 2.42 floor.
- These artifacts are **not signed**: checksums verify integrity, and
  `AUTHENTICITY_VERIFIED=NO` still holds for the 0.9.2 release.
