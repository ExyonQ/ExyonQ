# ExyonQ security audit — vX.Y.Z

| Field | Value |
|-------|-------|
| **Version** | X.Y.Z |
| **Commit** | `<full sha>` |
| **Date** | YYYY-MM-DD |
| **Previous audit** | [`audit-vA.B.C.md`](audit-vA.B.C.md) or N/A |

## Summary

One paragraph: overall posture, blockers, waivers count.

## Release gates

| Gate | Status | Notes |
|------|--------|-------|
| CI (`ci.yml`) | pass / fail | |
| Security (`security.yml`) | pass / fail | cargo audit, deny, security tests |
| Nightly fuzz | pass / fail / skipped | last run date |
| Functional F1–F13 | pass / fail | |
| WASM host tests | pass / fail / N/A | fuel/epoch traps |
| Benchmark Tier A | pass / fail / N/A | |

## Blockers

State `None` when no critical/high findings block release. Do not list waived or fixed items here.

## Findings (open)

Use format from [`review-checklist.md`](review-checklist.md). Write `None.` when empty.

### N. Title

- **Severidad:** crítica | alta | media | baja
- **Archivo:** `path:line`
- **Función:** `name`
- **Condición:** what triggers it
- **Impacto:** technical impact
- **Reproducción:** steps or test name
- **Patch mínimo:** suggested fix
- **Test:** regression test id
- **Estado:** open | waiver | fixed

## Findings (closed this release)

| ID | Severity | Resolution |
|----|----------|------------|
| | | |

## Waivers

| ID | Severity | Reason | Mitigation | Expires |
|----|----------|--------|------------|---------|
| | | | | |

## Surfaces reviewed

- [ ] HTTP/1.1 raw static loop
- [ ] Hyper proxy path
- [ ] Static path resolver
- [ ] Reverse proxy headers
- [ ] Config parse + reload + control socket
- [ ] TLS / HTTP/2
- [ ] HTTP/3 QUIC
- [ ] Discovery env overlays
- [ ] Modules (metrics, compression, ratelimit)
- [ ] `unsafe` sendfile / mmap

## Sign-off

- **Reviewer:** name
- **Date:** YYYY-MM-DD
- **Notes:**
