# ExyonQ security audit — vX.Y.Z

| Field | Value |
| --- | --- |
| **Version** | X.Y.Z |
| **Commit** | `<full sha>` |
| **Date** | YYYY-MM-DD |
| **Previous audit** | [`audit-vA.B.C.md`](audit-vA.B.C.md) or N/A |

> The **Commit** field names the audited code commit. If this document is
> added in a separate commit, that commit may be an audit-only seal: its first
> parent must be the audited commit and its only change must be this file. The
> annotated tag points at the seal; `scripts/verify-release-audit.sh` enforces
> both the direct and seal forms.

## Summary

One paragraph: overall posture, blockers, waivers count.

## Release gates

| Gate | Status | Notes |
| --- | --- | --- |
| CI (`ci.yml`) | pass / fail | |
| Security (`security.yml`) | pass / fail | cargo audit, deny, security tests |
| Nightly fuzz | pass / fail / skipped | last run date |
| Functional F1–F13 | pass / fail | |
| WASM host tests | pass / fail / N/A | fuel/epoch traps |
| Benchmark Tier A | pass / fail / N/A | |

## Wire and route parity

Mandatory evidence for every release that changes routing, auth, static serving or
request-path handling. Record the request, the credentials, the observable result and a
reproducible PoC. A status of `PASS` requires the real binary and a real origin; a unit
test alone is not this evidence.

| Case | Request | Expected | Observed | PoC |
| --- | --- | --- | --- | --- |
| Protected route, no credentials | `GET /admin` | `401` | | |
| Dot-segment alias | `GET /./admin` | `401` (not `200`) | | |
| Duplicate slash | `GET //admin` | `401` (not `200`) | | |
| Percent-encoded name | `GET /%61dmin` | `401` (not `200`) | | |
| Protected route, valid token | `GET /./admin` + token | `200` | | |
| Public route | `GET /open` | `200` | | |
| Traversal | `GET /../etc/passwd` | `400`/`403`, never `200` | | |
| Host-scoped route, no Host | `GET /` HTTP/1.1 without `Host` | `400` | | |
| Host-scoped route, correct Host | `GET /` + configured `Host` | `200` | | |

PoC command: `bash scripts/e2e/h01-l01-canonical-host-e2e.sh` (records the binary
SHA-256; `RESULT=PASS H01 L01`).

State the fallback used when a fast path declines: Hyper full dispatch is the
terminal owner for non-canonical paths, host-scoped requests without an
authority, routes carrying auth/cache/rewrite/WAF features and paths with no
static or proxy slot. Never leave a request in a terminal wire `404` merely
because a benchmark prefix did not match.

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
| --- | --- | --- |
| | | |

## Waivers

| ID | Severity | Reason | Mitigation | Expires |
| --- | --- | --- | --- | --- |
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
