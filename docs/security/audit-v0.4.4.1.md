# ExyonQ security audit — v0.4.4.1

| Field | Value |
|-------|-------|
| **Version** | 0.4.4.1 |
| **Commit** | `99bd7c547cb4d7c7821a1e131dfca5cf8d8db987` |
| **Date** | 2026-10-03 |
| **Previous audit** | [`audit-v0.4.4.md`](audit-v0.4.4.md) |

## Summary

v0.4.4.1 publishes the same product line as v0.4.4 with a scratch runtime image, corrected release identity, and public contact addresses. Wasmtime stays at 49.0.1. RUSTSEC-2026-0222 stays covered by the patched range `>=47.0.3`. No new waiver is opened.

The runtime image no longer ships Debian bookworm. That removes the Perl, pcre2, and copyleft operating-system packages that Docker Scout reported on `ghcr.io/exyonq/exyonq:0.4.4-arm64`.

## Release gates

| Gate | Status | Notes |
|------|--------|-------|
| CI (`ci.yml`) | inherited | No product logic change beyond version identity, docs, and the runtime image base |
| Security (`security.yml`) | inherited | Dependency pins are unchanged except the recorded product version |
| Docker Scout | expected pass | Scratch runtime; SPDX SBOM attestation required on the pushed image |
| Benchmark Tier A | not executed | No official benchmark claim |

## Blockers

None.

## Findings (open)

None.
