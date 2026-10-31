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

The old F1–F13 functional harness (`benchmarks/scenarios/functional/`) does not
exist in this tree — the directory only carries `lib.sh` — so the checklist
names the real commands that now provide that coverage.

- [ ] Full workspace suite green: `cargo nextest run --workspace` (includes
      `exyonq-integration-tests`).
- [ ] Path traversal: `cargo nextest run -p exyonq-core path_traversal_returns_403`
      and `cargo nextest run -p exyonq-mod-htaccess traversal_path_rejected_at_compile`.
- [ ] Upstream down / failover: `cargo nextest run -p exyonq-mod-proxy`
      (NG-07 wire affinity, NG-08 passive health, failover).
- [ ] TLS config validate + crypto policy:
      `cargo nextest run -p exyonq-config-ir -E 'test(/tls|ng12|ng13|ng24|ap02|ocsp|sni/)'`
      (14 tests) plus the `openssl s_client` battery below.
- [ ] Reload: `cargo nextest run -p exyonq-integration-tests security_reload_invalid_config_keeps_snapshot`
      and `cargo nextest run -p exyonq-reload-runtime`.
- [ ] Host E2E TLS battery on both Linux hosts (Netcup amd64, Oracle arm64):
      negotiated TLS 1.3 cipher matches `ciphers`, TLS 1.2 rejected when not
      configured, SNI multi-cert selects the right certificate, unknown SNI
      rejected with `strict_sni`, OCSP staple present, session resumed.
- [ ] WordPress image gate (rule 134) for the release tag on both architectures.

## 5. Integrity + private material

- [ ] `bash scripts/gates/project-integrity-gate.sh --selftest` and `--tree .` PASS.
- [ ] `bash scripts/gates/benchmark-integrity-gate.sh --selftest` and `--tree .` PASS.
- [ ] `bash scripts/gates/data-provenance-gate.sh --selftest` PASS (and `--check` on any result package used for claims).
- [ ] `bash scripts/security/scan-private-material.sh --git-tree --repo .` PASS.
- [ ] `bash scripts/gates/unsafe-allowlist-gate.sh --tree .` PASS (ADR-045 ratchet).
- [ ] `bash scripts/gates/publication-privacy-gate.sh` PASS on release material (rule 135).
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
