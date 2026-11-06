# ExyonQ security audit — v0.9.1

| Field | Value |
| --- | --- |
| **Version** | 0.9.1 |
| **Commit** | `3870648ca57a039d3ecc81c10d1ab9033dbd34ca` |
| **Date** | 2026-11-06 |
| **Previous audit** | [`audit-v0.9.0.md`](audit-v0.9.0.md) |

## Summary

v0.9.1 closes the four release requirements AP-04 (external JWT/OAuth auth
endpoint), OL-03 (OpenLiteSpeed per-client connection throttling), OL-04
(challenge with deploy-time configuration and no-JS fallback) and OL-05 (OWASP
CRS/ModSecurity differential in GitHub Actions). It also makes the README
versionless and prepares the server and WordPress images on Wolfi bases. The
public CI, Security and CRS workflows are green on the release candidate. The
CRS run used the pinned corpus and a real ModSecurity/CRS oracle; no mock or
fixture-only result is counted as evidence. No open Critical or High finding is
known for this increment. Docker Scout reports 0 Critical, 0 High, 0 Medium and
0 Low on the locally built arm64 server and WordPress images; the final
multiarch GHCR digests are scanned after publication and are not presented here
as already scanned. F6-WP-04 and the F6 DoD remain open and outside the four
requirements of this release.

## Release gates

| Gate | Status | Notes |
| --- | --- | --- |
| CI (`ci.yml`) | pass | run [`42242829709`](https://github.com/ExyonQ/ExyonQ/actions/runs/42242829709): `cargo fmt --check`, Clippy, **2319 passed / 2 skipped**, `semver-checks`, `supply-chain`, `wasm-host-check`, `static-linux-arm64` all green |
| Security (`security.yml`) | pass | run [`42242829750`](https://github.com/ExyonQ/ExyonQ/actions/runs/42242829750): `private-material` (including the clean-worktree hook regression), `legal-compliance`, `deps-audit`, `integrity-gates`, `functional-security-check`, `security-tests` |
| CRS differential (OL-05) | pass | run [`42242829711`](https://github.com/ExyonQ/ExyonQ/actions/runs/42242829711), 5m59s: block `failures=0/4628`, `excluded_with_evidence=33/4628`; dry-run `failures=0/4628`, `excluded_with_evidence=0/4628`, `would_block=5947`, `violations_942100=52`, `violations_949110=5947` |
| `cargo audit` | pass | `security.yml` `deps-audit` |
| `cargo deny check` | pass | `ci.yml` `supply-chain` (advisories/bans/sources) |
| Security functional tests | pass | `security-tests` on the same Security run |
| Private-material-zero | pass | publication scan, history scan, selftest and publication-hook regression on the same Security run |
| AP-04 / OL-03 / OL-04 | pass | real local E2E: AP-04 **30/30**, OL-03 **25/25**, OL-04 **11/11**; evidence indexed in `docs/releases/0.9.1-requirements-evidence.md` |
| OL-05 local | pass | `OL05_VERIFY=PASS`; CRS v4.25.0, corpus SHA-256 `1fb44865…`, real oracle digest; details in `docs/waf/ol05-evidence-20261105.md` |
| Docker Scout (local arm64 review) | pass | server and WordPress local images: 0 Critical / 0 High / 0 Medium / 0 Low; final multiarch digests pending post-publish scan |
| WordPress image review (local arm64) | pass | Wolfi base, PHP 8.5.12, `www-data`, installer endpoint HTTP `200`; full installer/login E2E on the final published digest not executed here |
| Nightly fuzz / Miri | not executed | not re-run for this release increment; not claimed green |
| Veyra Proof / rust-ai-audit / CodeGraph | not executed | audit-stack legs were not run in this release-document increment; reported as NOT EXECUTED, not as pass |
| F6-WP-04 / DoD F6 | open | technical gate PASS on the 0.9.0 tree; editorial/production closure and part of the F6 DoD remain open; not claimed closed |

## Blockers

None.

## Findings (open)

None.

## Findings (closed this release)

| ID | Severity | Resolution |
| --- | --- | --- |
| AP-04 | — | External JWT/OAuth endpoint with internal-header isolation; 30/30 local E2E |
| OL-03 | — | Per-client connection throttle migrated and compared against OLS; 25/25 local E2E |
| OL-04 | — | Native/reCAPTCHA/hCaptcha challenge and no-JS fallback; 11/11 local E2E |
| OL-05 | — | Real OWASP CRS differential and dry-run in GitHub Actions; ruled |

## Waivers

| ID | Severity | Reason | Mitigation | Expires |
| --- | --- | --- | --- | --- |
| — | — | — | — | — |

## Surfaces reviewed

- [x] External auth boundary and protected-header injection (AP-04)
- [x] Connection admission/throttling and hard-close behavior (OL-03)
- [x] Challenge issuance, provider configuration and no-JS path (OL-04)
- [x] WAF/CRS parsing, detection-only mode and per-rule metrics (OL-05)
- [x] Release identity, `source_tag` embedding and version coherence
- [x] Release publication gates and private-material controls

## Docker image review

- **Server** — local `exyonq:0.9.1-local`, Wolfi/Chainguard build and runtime
  bases, non-root `exyonq` entrypoint; `--version` reports product/artifact
  `0.9.1` and `source_tag=v0.9.1`. Scout: 0C/0H/0M/0L (arm64).
- **WordPress** — local `exyonq-wordpress:0.9.1-local`, Wolfi base, PHP
  8.5.12, WordPress + SQLite payload, runs as `www-data`; the installer page
  returned HTTP `200`. Scout: 0C/0H/0M/0L (arm64).
- The source revisions in those local builds predate the release tag; the
  release workflow rebuilds both images from the tag and publishes a
  multiarch index. The final GHCR digests still need their own Scout scan and
  the amd64/arm64 runtime E2E; this audit does not claim those as done.

## Process limitations (NOT EXECUTED)

- Veyra Proof and rust-ai-audit were not run in this release-document
  increment. No ACCEPTED verdict is claimed.
- CodeGraph indexing/impact was not run. No graph result is claimed.
- Hindsight retention was not available in this session. No memory write is
  claimed.
- Nightly fuzz, Miri, the official sequential benchmark and independent
  external audits were not run for this release increment. They are NOT
  EXECUTED, not PASS.
- F6-WP-04 editorial/production closure and the remaining F6 DoD items are
  outside this release's four requirements and are still open.

## Sign-off

- **Reviewer:** release pipeline (gates above; reproducible run IDs and public
  evidence paths)
- **Date:** 2026-11-06
- **Notes:** No open Critical/High findings are known for this increment.
  Missing audit-stack legs and the final multiarch image checks are reported
  as limitations, not as passes. The tag is annotated but not signed; no
  signature claim is made.
