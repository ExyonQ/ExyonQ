# P14V042 — Dependency decisions (v0.4.2)

```text
DOCUMENT = docs/releases/p14v042-dependency-decisions.md
P14V042_DEPENDENCY_GATE = PASS
GLOBAL_LOCKFILE_REFRESH = NO
```

Scope authority: `docs/releases/p14v042-scope-audit.md`  
Execution: `docs/releases/p14v042-execution.md`

---

## Decisions

| Dependency | Before | After | Status | Reason |
|---|---|---|---|---|
| rustls-pemfile | 2.2.0 direct | **removed** | `REMOVE_UNUSED` / `UPDATED_V042` | OPTION_A via `rustls-pki-types` PEM; PKCS#8-only preserved; RUSTSEC-2025-0134 gone from lock |
| serde_json | 1.0.150 | **1.0.151** | `UPDATED_V042` | admitted patch |
| redis | 0.27.6 | **0.32.7** | `UPDATED_V042` | lowest maintained line with `socket2 ^0.6`; not 1.x |
| socket2 0.5.10 | transitive via redis | **absent** | `UPDATED_V042` | removed as consequence of redis 0.32.7 |
| socket2 0.6.5 | present | present | `ALREADY_LATEST_COMPATIBLE` | sole remaining socket2 line |
| rustls | 0.23.40 | 0.23.40 | `DEFERRED_WITH_TECHNICAL_REASON` | audit allows defer unless pulled by TLS work; dual-arch TLS smoke cost not justified for patch-only stack bump in this train |
| matchit | 0.8.x | unchanged | `DEFERRED_WITH_TECHNICAL_REASON` | routing major 0.8→0.9 — not maintenance-min |
| wasmtime | =45.0.2 | unchanged | `DEFERRED_WITH_TECHNICAL_REASON` | rule 120 / ABI — not drive-by |
| quinn | 0.11.11 | unchanged | `ALREADY_LATEST_COMPATIBLE` / defer | no drive-by |
| io-uring | platform area | unchanged | `BLOCKED` / deferred | needs dedicated dual-arch + freeze policy |
| criterion | 0.5.x | unchanged | `DEFERRED_WITH_TECHNICAL_REASON` | dev-only |
| GitHub Actions | SHA-pinned (0.4.1) | unchanged | `ALREADY_LATEST_COMPATIBLE` | no pin drift / no permission widening required for v0.4.2 |

Forbidden status used: **none** (`DEFERRED_FOR_CONVENIENCE` not used).

---

## Redis selection note

```text
FIRST_SOCKET2_0_6_REDIS = 0.32.7
REDIS_1_X = NOT_SELECTED
REASON = lowest maintained compatible line that eliminates socket2 0.5 without 1.x migration cost
SOURCE_API_CHANGES = NONE for exyonq-cache-redis (compile-compatible)
```

---

## Evidence hosts

```text
Netcup amd64 = PASS (unit + integration with Redis)
Oracle arm64 = PASS (unit + integration with Redis)
```
