# V044 — Phase 7 Static + Proxy + FastCGI Requalification

**WIP:** `V044_PHASE7_STATIC_PROXY_FASTCGI_REQUALIFICATION`  
**Parent terminal:** `P7STATICIMPL-B` (native CFD Static implementation)  
**Supersedes pre-Static Phase 7 attempt:** `P7-E` (Static handler absent — historical only)  
**Platform authority:** Linux amd64 Netcup (`netcup-bench`)  
**PRODUCT_MUTATION this WIP:** NO (evidence harness + docs only)

---

## Primary question (answered)

Can **current** ExyonQ CFD serve **real native Static**, **real reverse Proxy**, and **real FastCGI (PHP-FPM)** **simultaneously** inside **one CFDRT005 generation**, with Cap033 route ownership, no cross-handler contamination, isolated failures, and safe reload?

**Answer (authoritative run):** **YES** — bounded Netcup amd64 proof with nonblocking qualifications preserved from prior static work.

---

## Terminal disposition

| Field | Value |
|---|---|
| **CASE** | **P7R-B** — `STATIC_PROXY_FASTCGI_COEXISTENCE_VERIFIED_WITH_NONBLOCKING_QUALIFICATIONS` |
| **Authoritative RUN_ID** | `20260901T140000Z` |
| **CORE_MATRIX** | PASS (`CORE_MATRIX_FAILURES=0`) |
| **BINARY_SHA256** | `6d9bc14db019633ecbd3519ea8cb8e78ce11a76abaedb0d039f95bb9ad8c413e` |
| **PHASE7_STATUS** | `CLOSED_WITH_NONBLOCKING_QUALIFICATIONS` |

Invalid harness runs (`20260901T132500Z`, `20260901T134500Z`) are **not** authoritative — see evidence ledger.

---

## Lineage (immutable preserved truths)

| Terminal | Preserved claim |
|---|---|
| **P7STATICIMPL-B** | Native `BackendKind::Static`, CFDRT005, StaticSend, Netcup static matrix |
| **P7-E** | Pre-Static: Proxy+FastCGI only; Static **falsified absent** — do not reuse as pass |
| **P6G-A** | Real FastCGI / KEEP_CONN contract (`max_conn=1`) |
| **CAP067** | Native static product plane — no remingle |

**Not silently closed** (from P7STATICIMPL-B): symlink TOCTOU residual, pread fallback runtime, EAGAIN/partial-sendfile adversarial quals, positive in-root symlink serve on AMD64.

**Dual-arch Static (separate WIP):** [v044-static-dual-arch-qualification.md](v044-static-dual-arch-qualification.md) — **STATICDUAL-B**; native Oracle ARM64 Static matrix PASS (`20260901T171500Z`); in-root symlink **PASS on ARM64**; P7R-B three-handler same-gen remains Netcup amd64-only.

---

## Topology (one generation)

Published in **G1** (`CONFIG/routes_g1.txt`):

```text
static|1|${DOCROOT}|index.html
|/static|static:1
|/assets|static:1
fcgi|1|tcp:127.0.0.1:${FPM_PORT}|${APP_ROOT}|1|...
fcgi-front-controller|1|/index.php
|/app/|fcgi:1
|/api/|127.0.0.1:${UP_PORT}|127.0.0.1
```

- **Static:** CFD opens/serves filesystem objects (SHA256 oracles on `hello.txt`, `binary.bin`, `large.bin`).
- **Proxy:** Real Python `ThreadingTCPServer` upstream (`X-P7R-Handler: PROXY`, side-effect counter).
- **FastCGI:** Real PHP-FPM + minimal front-controller (`X-P7R-Handler: FASTCGI`).

**Forbidden:** proxy-to-file-server as Static proof.

---

## Authoritative evidence summary (`20260901T140000Z`)

| Gate | Result |
|---|---|
| SAME_GENERATION_STATIC_PROXY_FASTCGI | PASS |
| EXACTLY_ONE_HANDLER_OWNS_REQUEST | PASS |
| THREE_WAY_INTERLEAVING | PASS (`ROUTE_CROSS_CONTAMINATION=MEASURED_0`) |
| MIXED_CONCURRENCY (C=1/8/32) | PASS |
| Static matrix + security regression | PASS |
| Proxy matrix + failure/recovery | PASS |
| FastCGI matrix + failure/recovery | PASS |
| MIXED_GENERATION_RELOAD / FAILED_PUBLISH | PASS |
| HANDLER_CONFUSION | 0 |
| Resource sanity (FD/RSS/sockets) | PASS bounded |

Evidence root: `.exyonq-local/evidence/phase7-static-proxy-fastcgi-requalification/20260901T140000Z/`

Harness: `scripts/reality/run-phase7-static-proxy-fastcgi-requalification-netcup.sh`  
Remote driver: `scripts/remote/v044-phase7-static-proxy-fastcgi-requalification-netcup.sh`

---

## Nonblocking qualifications (P7R-B)

1. **Static residual reality** — P7STATICIMPL-B qualifications unchanged (TOCTOU, dual-arch, adversarial I/O).
2. **STALE_GENERATION_ROUTE_CROSSOVER** — `NOT_MEASURED_WITH_REASON` (no in-flight crossover oracle).
3. **FASTCGI_DUPLICATE_EXECUTION** — `PRESERVED_NOT_MEASURED_SIDE_EFFECT` (no DB side-effect oracle in fixture app).
4. **SLOW_FASTCGI_ISOLATION** — `PASS_EXPLICIT_ARCHITECTURAL_QUALIFICATION` (single-shard synchronous FCGI; not async multi-inflight).
5. **Platform scope** — Linux amd64 Netcup only; not dual-arch production close.
6. **Security / architecture auditors** — CONDITIONAL (bounded TOCTOU residual; no proven reachable confusion defect).

---

## Specialist audits (HEAD `d94856663f42ffce16a3c0ccc1291feb4d7083e4`)

| Auditor | Verdict |
|---|---|
| Vertex architect | CONDITIONAL_ACCEPT |
| Security engineer | CONDITIONAL (`MUST_FIX=0`) |
| Research engineer | CONDITIONAL (credible migration topology class) |
| GRC engineer | PASS (matrix reconciled; invalid runs ledgered) |
| Logic auditor | PASS; `P7_REQUALIFICATION_FALSIFIED=NO`; `SUPPORTED_CONTRACT_MUST_FIX_COUNT=0` |

---

## Workspace gates (qualified scope)

Ambient worktree dirt prevents full-workspace attribution. **CFD-scoped gates PASS:**

- `cargo check` / `clippy` / `cargo test` — `exyonq-cfd-{gen,control,dataplane}`
- `phase7_static_native` integration (2/2)
- `phase6b_fcgi` (5/5)
- `route_table` static+fcgi tests (12/12)

See `GATES/workspace_gates.txt` in evidence package.

---

## Migration topology value

**DOES_P7_REQUALIFICATION_CLOSE_MIGRATION_TOPOLOGY_GAP = PARTIAL**

One CFD process can host the common **LEMP+API** split (static assets + `/api/` reverse proxy + PHP `/app/`) with reload and tri-handler isolation — the gap P7-E could not close because Static was absent. Remaining migration gaps: dual-arch evidence, production app fixtures (Laravel/WordPress under this topology), static MVP completeness (Range/ETag/cache), async FastCGI architecture admission.

---

## Recommended next WIP (not authorized)

**`STATIC_DUAL_ARCH_QUALIFICATION`** or **`PHASE8_COMPLETENESS_AND_ADVERSARIAL_SECURITY`** — owner to authorize.

`NEXT_WIP_AUTHORIZED=NO` · `PHASE_8_AUTHORIZED=NO` · `COMMIT/PUSH/TAG/RELEASE=NO`

**Later event (does not rewrite CASE P7R-B):** Phase 8 later authorized and terminated **P8-C** — [v044-phase8-completeness-and-adversarial-security.md](v044-phase8-completeness-and-adversarial-security.md). `PHASE_8_AUTHORIZED=NO` above is the P7R-B seal, not the later owner message.

---

## Related documents

- Terminal block: [v044-phase7-static-proxy-fastcgi-requalification-terminal-block.txt](v044-phase7-static-proxy-fastcgi-requalification-terminal-block.txt)
- Prior Phase 7 (Static absent): [v044-phase7-static-proxy-coexistence.md](v044-phase7-static-proxy-coexistence.md) — **P7-E**
- Static implementation: [v044-cfd-native-static-filesystem-handler-implementation.md](v044-cfd-native-static-filesystem-handler-implementation.md) — **P7STATICIMPL-B**
