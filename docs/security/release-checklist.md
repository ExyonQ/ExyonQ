# Release security checklist

Complete before tagging `vX.Y.Z` (~30 minutes).

## 1. Diff review

- [ ] Review changes since previous tag: parsers, `unsafe`, reload, TLS/QUIC, control socket, modules.
- [ ] No new `unwrap()` / `expect()` on untrusted input paths without justification.

## 2. Dependencies

- [ ] Dependabot PRs for critical/high advisories merged or documented in audit waivers.
- [ ] `cargo xtask security check` passes locally.

## 3. Automated analysis

- [ ] Nightly fuzz green in last 24h, or run `scripts/security-fuzz.sh` locally.
- [ ] Miri job green (Linux) or no new `unsafe` without Miri coverage.

## 4. Functional

- [ ] `bash benchmarks/scenarios/functional/run-all.sh` — F1–F13 pass for ExyonQ.
- [ ] Path traversal (F4), upstream down (F7), TLS validate (F9), reload (F11) verified.

## 5. Integrity + private material

- [ ] `bash scripts/gates/project-integrity-gate.sh --selftest` and `--tree .` PASS.
- [ ] `bash scripts/gates/benchmark-integrity-gate.sh --selftest` and `--tree .` PASS.
- [ ] `bash scripts/gates/data-provenance-gate.sh --selftest` PASS (and `--check` on any result package used for claims).
- [ ] `bash scripts/security/scan-private-material.sh --git-tree --repo .` PASS.
- [ ] Tracked waivers under `docs/governance/exceptions/` complete with expiry (`waiver-expiry-gate.sh --check`).

## 6. Audit artifact

- [ ] Copy [`audit-template.md`](audit-template.md) → `audit-vX.Y.Z.md` (or `cargo xtask security audit --version X.Y.Z`).
- [ ] Fill version, commit, gates, findings, waivers, surfaces, sign-off.
- [ ] No unresolved entries under `## Blockers`.
- [ ] `bash scripts/verify-release-audit.sh X.Y.Z` PASS.
- [ ] Optional: independent security review of the release diff before sign-off.

## 7. Tag and release (owner ceremony only)

- [ ] Signing readiness reviewed ([signing-readiness.md](../governance/signing-readiness.md)) — production keys **outside** repo.
- [ ] `git tag -s -a vX.Y.Z` on audited commit (SSH tag signing).
- [ ] Cosign blob + OCI digest signing per `security/signing/` (manual owner ceremony).
- [ ] `release.yml` `security-release` job green.
- [ ] Update [SECURITY.md](../../SECURITY.md) supported versions / current audit link.

## 7. Post-release

- [ ] Link audit from SECURITY.md “Current audit”.
- [ ] File issues for deferred low-severity items.
