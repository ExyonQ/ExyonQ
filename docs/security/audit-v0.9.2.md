# ExyonQ security audit — v0.9.2

| Field | Value |
| --- | --- |
| **Version** | 0.9.2 |
| **Commit** | `64b8388d4673ef89bbd2fe7b238eaa87b43c568c` |
| **Date** | 2026-11-18 |
| **Previous audit** | [`audit-v0.9.1.md`](audit-v0.9.1.md) |

> The **Commit** field names the audited code commit. The commit that adds this
> document is an audit-only seal: its first parent is `64b8388`, its only change
> is this file, and the annotated tag `v0.9.2` points at the seal.
> `scripts/verify-release-audit.sh` enforces that binding.

## Summary

This audit covers the 0.9.2 release tree at commit
`64b8388d4673ef89bbd2fe7b238eaa87b43c568c`. It contains the wire/Hyper parity
work, canonical request-path handling, route-aware wire eligibility without
benchmark prefixes, per-route request-body limits, WAF default hardening, the
WordPress cache plugin lifecycle, config v1 to v2 migration and the L-03 audit
binding, plus one test-isolation fix (T-01 below) found by this audit.

Veyra `standard` returned **ACCEPTED**; `cargo-geiger` remained
**INCONCLUSIVE** (coverage warnings; unsafe counts are a lower bound) and is not
reported as a pass. Rust Audit 0.2.9 over a clean export of the audited commit
reports the pre-existing repository-wide critical/high counts listed below; the
critical hits were reviewed one by one and none is an open release blocker.
CodeGraph fast-tier indexing completed over the audited tree. The native
amd64 and arm64 parity matrices both recorded `RESULT=PASS H01 L01` (12/12)
against the audited source.

The release workflows, the GHCR/Docker Hub images and the artifact signatures
are **NOT EXECUTED** at seal time: the seal is created before publication, and
their results are recorded in [`docs/release/v0.9.2.md`](../release/v0.9.2.md)
after the tag runs. No open blocker attributable to this commit is known.

## Release gates

| Gate | Status | Notes |
| --- | --- | --- |
| Native Linux amd64 parity (H-01/L-01) | pass | `RESULT=PASS H01 L01`, `pass=12 fail=0`. Binary sha256 `73f0866edba79677a9c5063de9c2171f6486effde05c28c681851febb7c1f3e3`. Run `run-20261118T211930Z`; log `.exyonq-local/evidence/h01-l01-092-publish/amd64-stdout.log`. |
| Native Linux arm64 parity (H-01/L-01) | pass | `RESULT=PASS H01 L01`, `pass=12 fail=0`. Binary sha256 `3e72f345f6466ed052121c033581d16c643108e0405f2d37ade3bd768d09972e`. Run `run-20261118T211935Z`; log `.exyonq-local/evidence/h01-l01-092-publish/arm64-stdout.log`. |
| Veyra Proof (`standard`) | accepted | `Decision: ACCEPTED`. OK: rustfmt, cargo check, clippy, cargo test, cargo-audit, veyra-anti-theater, cargo-nextest, cargo-llvm-cov, cargo-deny, cargo-machete. INCONCLUSIVE: `cargo-geiger` (47 coverage warnings; unsafe counts are a lower bound, not a pass). SKIPPED: dylint (no lint config), cargo-semver-checks (no registry baseline), veyra-e2e (no `[[e2e.scenarios]]`). Evidence `.veyra/evidence/20261118T210257Z/report.md`. |
| Rust Audit 0.2.9 (clean export of `64b8388`) | reviewed | Exit 1 by `--fail-on high`. 23 critical / 115 high / 946 medium / 410 low / 7 info. Every critical reviewed: 1 deliberate must-fail fixture (`scripts/gates/fixtures/must-fail/path-cache/bad.rs:6`) and 22 classifier false positives (high-entropy header/cipher/needle string literals in `redaction.rs:85-91`, `exyonq-mod-tls/src/lib.rs:493/502`, `core/src/server/handler.rs:67`, `bounds.rs`, `diagnostic.rs:528`, `lib.rs:3512/3518`, `hub.rs:466/557`, `static_dispatch.rs:306`, and a stale offline `dashmap` version claim against `Cargo.toml:1`). Report `report/rust-audit-20261118-220345.md` in the clean export (local, not tracked). |
| CodeGraph index (`fast` tier) | pass | 663 files, 215,269 lines, 25,546 nodes, 30,488 edges, resolved 36.2%, embeddings 0 (`CODEGRAPH_EMBEDDING_POLICY=off CODEGRAPH_SEMANTIC_RESOLUTION=off`), 94.9 s. Log `cg5.log` (local, not tracked). Agentic queries **NOT EXECUTED** (embeddings disabled by policy). |
| CI (`ci.yml`) / Security (`security.yml`) | not executed | No workflow run at seal time; the release workflow runs after the tag and its result is reported in `docs/release/v0.9.2.md`. |
| Release workflow, GHCR/WordPress images, Docker Hub mirror | not executed | Not built or published before the seal; verified post-publication by digest. |
| Tag and artifact signatures | not executed | The tag is annotated; signing is a separate claim recorded after publication. |

## Wire and route parity

Evidence: `.exyonq-local/evidence/h01-l01-092-publish/{amd64,arm64}-stdout.log`
and the per-case logs in the referenced `run-*` directories. Both hosts ran
`scripts/e2e/h01-l01-canonical-host-e2e.sh` against the real `exyonq` binary
built from the audited tree, a real auth endpoint on `127.0.0.1:19181` and a
real origin on `127.0.0.1:19182`; the binary SHA-256 is recorded in each log.

| Case | Request | Expected | Observed | PoC |
| --- | --- | --- | --- | --- |
| Protected route, no credentials | `GET /admin` | `401` | `401` | `scripts/e2e/h01-l01-canonical-host-e2e.sh` |
| Dot-segment alias | `GET /./admin` | `401` (not `200`) | `401` | same |
| Duplicate slash | `GET //admin` | `401` (not `200`) | `401` | same |
| Percent-encoded name | `GET /%61dmin` | `401` (not `200`) | `401` | same |
| Protected route, valid token | `GET /./admin` + token | `200` | `200` | same |
| Canonical body reaches origin | `GET /./admin` + token | `origin:/admin` | `origin:/admin` | same |
| Public route | `GET /open/index.html` | `200` | `200` | same |
| Traversal | `GET /../etc/passwd` | `400`/`403`, never `200` | `403` | same |
| Traversal, encoded | `GET /%2e%2e/etc/passwd` | `400`/`403`, never `200` | `403` | same |
| Host-scoped route, no Host | `GET /` HTTP/1.1 without `Host` | `400` | `400` | same |
| Host-scoped route, correct Host | `GET /` + configured `Host` | `200` | `200` | same |

The raw paths decline to Hyper full dispatch for non-canonical spellings,
host-scoped requests without an authority, routes with auth/cache/rewrite/WAF
features, and paths without a static or proxy slot. Hyper full dispatch remains
the terminal owner for those requests; no request ends in a wire `404` merely
because a benchmark prefix did not match.

## Blockers

None

## Findings (open)

None.

## Findings (closed this release)

| ID | Severity | Resolution |
| --- | --- | --- |
| T-01 | medium (test isolation) | The regression test `server::run_on_pins_listen::serves_the_given_address_when_the_configured_listen_is_busy` starts a real server without the shared `WIRE_HOOKS_GATE`. Applying a config re-binds the process-global proxy cluster table and bumps its generation, so a sibling wire/parity test serving at the same time could receive an unrelated `503 Service Unavailable`. Veyra's first `standard` run on `838c487` caught it as `cargo-test:test-failure` (`ng11_disabled_ignores_xff`). Fixed in `64b8388` by holding the gate for the test's duration; the full `cargo test -p exyonq-core --lib` suite (252 tests) then passed six consecutive runs, and the re-run Veyra audit is the one cited above. |
| L-03 | — | The audit document is bound to the code commit; the seal commit changes only this file and the tag points at the seal. |

## Waivers

| ID | Severity | Reason | Mitigation | Expires |
| --- | --- | --- | --- | --- |
| — | — | — | — | — |

## Surfaces reviewed

- [x] HTTP/1.1 raw static loop and path canonicalization
- [x] Hyper request path and proxy forwarding parity
- [x] Static/proxy wire eligibility without benchmark prefixes
- [x] Per-route request-body and WAF inspection bounds
- [x] Config IR v1 to v2 migration
- [x] WordPress cache plugin lifecycle and purge token reuse
- [x] Release-audit commit and tag binding
- [x] Test isolation of process-global module runtimes
- [ ] CI/Security workflows, release images, Scout and signature gates (NOT EXECUTED before the seal)

## Publication limitations

- The seal precedes publication by construction: no GitHub Actions CI, Security,
  CRS or E2E workflow, no GHCR/Docker Hub image and no signature exists at seal
  time. Those gates are executed by the tag workflow and their real results are
  recorded in `docs/release/v0.9.2.md`.
- Veyra's `cargo-geiger` leg is INCONCLUSIVE, not PASS; `dylint`,
  `cargo-semver-checks` and `veyra-e2e` are SKIPPED for the reasons Veyra
  records.
- CodeGraph agentic impact/context queries are NOT EXECUTED; only the index
  statistics are claimed.
- The parity matrices run debug binaries built from the audited tree; the
  published release archives are exercised by digest after publication.

## Sign-off

- **Reviewer:** local release pipeline (commands and paths above; no external
  reviewer)
- **Date:** 2026-11-18
- **Notes:** Local audit seal. Missing post-publication gates are reported as
  NOT EXECUTED here and verified in `docs/release/v0.9.2.md`, not as passes.
