# ExyonQ security audit — v0.9.2

| Field | Value |
| --- | --- |
| **Version** | 0.9.2 |
| **Commit** | `cf6b7b342375b921d78ccb71e59863817e9bc26d` |
| **Date** | 2026-11-08 |
| **Previous audit** | [`audit-v0.9.1.md`](audit-v0.9.1.md) |

## Summary

This audit covers the local 0.9.2 release candidate at commit
`cf6b7b342375b921d78ccb71e59863817e9bc26d`. The candidate contains the
wire/Hyper parity work, canonical request-path handling, route-aware wire
eligibility without benchmark prefixes, per-route request-body limits, WAF
default hardening, the WordPress cache plugin lifecycle, config v1 to v2
migration, and the L-03 audit binding. Veyra standard returned `ACCEPTED`;
`cargo-geiger` remained **INCONCLUSIVE** and is not reported as a pass. The
native Linux amd64 and arm64 matrix passed the changed journeys. Rust Audit
still reports pre-existing repository-wide critical/high counts; the two
findings on lines added by this increment were reviewed as invariant/test-only
`.expect` false positives. Full CI, Security, release-image and signature
qualification were **NOT EXECUTED** for this candidate. This document is a
local audit seal, not publication approval. No open blocker attributable to
this increment is known.

## Release gates

| Gate | Status | Notes |
| --- | --- | --- |
| Native Linux amd64 matrix | pass | H-01/L-01 `RESULT=PASS H01 L01` (12/12), M-02 parity, L-02 gate and affected package tests; local evidence under `.exyonq-local/evidence/native-matrix-092/amd64/`. |
| Native Linux arm64 matrix | pass | H-01/L-01 `RESULT=PASS H01 L01` (12/12), M-02 parity, L-02 gate and affected package tests; local evidence under `.exyonq-local/evidence/native-matrix-092/arm64/`. |
| Veyra Proof (`standard`) | accepted | `Decision: ACCEPTED`; rustfmt, cargo check, clippy, cargo test, nextest, llvm-cov, audit, deny and machete OK. `cargo-geiger` INCONCLUSIVE (54 coverage warnings, lower-bound unsafe counts) and is not a PASS. Local report: `.veyra/evidence/20261108T131728Z/report.md`. |
| Rust Audit final3 | reviewed | Exit 1 by `--fail-on high`. 33 critical / 402 high / 753 medium / 407 low / 7 info, mostly pre-existing repository-wide classifier hits. The two hits on added lines (`config/ir/src/canonical.rs:122`, `core/src/kernel/test_hooks.rs:77`) are `.expect` invariant/test-only false positives. `static_files.rs` has zero hits. Full triage of pre-existing findings was not re-run. |
| CodeGraph index | partial | Isolated DB index completed: 641 files, 207,705 lines, 24,612 nodes, 29,620 edges, resolved 36.4%, embeddings 0. Agentic impact/context queries were **NOT EXECUTED** because the Ollama embedding tokenizer is unavailable. Log: `.exyonq-local/evidence/codegraph-092-final/index-alt.log`. |
| Private-material publication scan | pass | `PUBLICATION_FILES=1338`, `PUBLICATION_INDEX_FILES=1320`, `PRIVATE_MATERIAL_FINDINGS=0`, `PRIVATE_MATERIAL_GATE=PASS`; log `.exyonq-local/evidence/release-092/private-material.log`. |
| Release build (`cargo build --workspace --release --locked`) | not executed | The first attempt was interrupted by the terminal timeout and produced no new `target/release/exyonq`; no exit code is claimed. |
| CI and Security workflows | not executed | No push or workflow run was authorized. |
| GHCR/WordPress images, Docker Scout | not executed | No image was built or published. |
| SSH/cosign artifact and tag signatures | not executed | The local tag is annotated but unsigned; no signature is claimed. |

## Wire and route parity

Evidence: `.exyonq-local/evidence/native-matrix-092/{amd64,arm64}/h01-l01.log`.
Both hosts recorded `RESULT=PASS H01 L01` and rechecked the binary SHA-256.

| Case | Request | Expected | Observed | PoC |
| --- | --- | --- | --- | --- |
| Protected route, no credentials | `GET /admin` | `401` | `401` | `scripts/e2e/h01-l01-canonical-host-e2e.sh` |
| Dot-segment alias | `GET /./admin` | `401` (not `200`) | `401` | same |
| Duplicate slash | `GET //admin` | `401` (not `200`) | `401` | same |
| Percent-encoded name | `GET /%61dmin` | `401` (not `200`) | `401` | same |
| Protected route, valid token | `GET /./admin` + token | `200` | `200` | same |
| Public route | `GET /open/index.html` | `200` | `200` | same |
| Traversal | `GET /../etc/passwd` | `400`/`403`, never `200` | `403` | same |
| Traversal, encoded | `GET /%2e%2e/etc/passwd` | `400`/`403`, never `200` | `403` | same |
| Host-scoped route, no Host | `GET /` HTTP/1.1 without `Host` | `400` | `400` | same |
| Host-scoped route, correct Host | `GET /` + configured `Host` | `200` | `200` | same |

The raw paths decline to Hyper full dispatch for non-canonical spellings,
host-scoped requests without an authority, routes with auth/cache/rewrite/WAF
features, and paths without a static or proxy slot.

## Blockers

None

## Findings (open)

No open blocker attributable to this increment. The pre-existing Rust Audit
counts and the two reviewed false positives are recorded above; no claim is
made that the whole repository has been re-triaged.

## Findings (closed this release)

| ID | Severity | Resolution |
| --- | --- | --- |
| L-03 | — | Audit document is bound to the code commit; the seal commit changes only this file and the tag points at the seal. |

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
- [ ] Full CI/Security, release images, Scout and signature gates (NOT EXECUTED)

## Publication limitations

- No `cargo build --workspace --release --locked` exit code exists for this
  candidate; the attempt was interrupted by the terminal timeout.
- No GitHub Actions CI, Security, CRS or E2E workflow ran on this commit.
- No GHCR or Docker Hub image was built, scanned or published.
- Veyra's `cargo-geiger` leg is INCONCLUSIVE, not PASS.
- CodeGraph agent queries are NOT EXECUTED; only the index statistics above
  are claimed.
- The tag is local and annotated but unsigned. Publication remains forbidden
  until the release checklist is completed and the owner authorizes it.

## Sign-off

- **Reviewer:** local release pipeline (commands and paths above; no external
  reviewer)
- **Date:** 2026-11-08
- **Notes:** Local audit seal only. Missing release gates are reported as NOT
  EXECUTED, not as passes.
