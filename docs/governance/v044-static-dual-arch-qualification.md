# V044 — Static dual-arch qualification (Linux AMD64 + ARM64)

```text
WIP = V044_STATIC_DUAL_ARCH_QUALIFICATION
PARENT_TERMINAL = P7R-B
STATIC_IMPLEMENTATION_PARENT = P7STATICIMPL-B
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
TERMINAL_EVIDENCE_RUN = 20260901T171500Z
EVIDENCE = .exyonq-local/evidence/static-dual-arch-qualification/20260901T171500Z/
PRODUCT_MUTATION = NO
HARNESS_MUTATION = YES_EVIDENCE_ONLY
```

## Question

Does the **current** native CFD Static handler preserve the same supported semantics on **native Linux ARM64 Oracle A1** as already verified on **Linux AMD64 Netcup**?

This WIP is **architecture/OS portability qualification only** — not Phase 7 rerun, not performance, not Phase 8.

## Terminal case

**STATICDUAL-B** — `NATIVE_STATIC_DUAL_ARCH_VERIFIED_WITH_NONBLOCKING_QUALIFICATIONS`

Core Static correctness and security are dual-arch proven on native Oracle ARM64 with byte-identical oracles, sendfile strace proof, in-root symlink serve (**PASS on ARM64**), and zero disclosure counters. Preserved nonblocking qualifications match P7STATICIMPL-B / P7R-B: symlink TOCTOU residual, pread fallback not runtime-proven, EAGAIN/partial sendfile not adversarially exercised, FIFO probe requires bounded harness dataplane restart (not a product repair).

**Does not** reopen P7R-B, P7STATICIMPL-B, P7-E, P6G-A, P6F-A, P6EROUTE-A, or CAP067.

## Source equivalence

Five CFD Static source files bound at entry; **content SHA-256** match across AMD64 reference digests (Darwin bind) and Oracle ARM64 tree:

| File | Content SHA-256 |
|------|-----------------|
| `route_table.rs` | `32aba8c9…` |
| `composite.rs` | `ba77af85…` |
| `static_serve.rs` | `b5d29047…` |
| `shard.rs` | `f727a387…` |
| `project.rs` | `61a7c3ac…` |

`AMD64_SOURCE_ID` = `ARM64_SOURCE_ID` = `09c5d58031633a930c1126b620b62546a21458da171e025811f78e5858261eba`  
`CROSS_ARCH_SOURCE_EQUIVALENCE` = **PASS** (content hashes; paths differ by host).

## Native ARM64 execution

| Field | Value |
|-------|-------|
| Host | `vnic-quasar` (Oracle Cloud A1) |
| `uname -m` | `aarch64` |
| Kernel | `6.17.0-1018-oracle` |
| Target triple | `aarch64-unknown-linux-gnu` |
| Build | `cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release` (native on Oracle) |
| Binary SHA-256 | `e5269a67eb78fdf37e61379f038f1605d60ffb7d4e347b1c9d31d38c2e272c3f` |

Reference AMD64 Netcup Static binaries remain those sealed under P7STATICIMPL-B / P7R-B — **different SHA-256 expected per architecture**.

## Authoritative matrix (Oracle ARM64)

Run `20260901T171500Z`: `CORE_MATRIX=PASS`. Highlights:

- GET small/binary/large + SHA oracles: **PASS**
- HEAD (0 body bytes): **PASS**
- MIME matrix (9 extensions): **PASS**
- Security: `.php`/dotfile/traverse/out-root symlink → **0 bytes disclosed**; out-root symlink **BLOCKED**
- In-root symlink `link.txt → real.txt`: **PASS** (positive vs AMD64-only terminal gap)
- sendfile: **PASS_REAL_LINUX** — strace `sendfile(…)=2097152` on 2 MiB `large.bin`
- Keepalive (curl `--next` sequence): **PASS**
- Generation reload G1→G2, invalid publish fail-closed: **PASS**
- FD leak measured **0** (baseline=peak=final=8)
- Three-handler CFDRT005 parse-only (not full P7 coexistence on ARM64): **PASS**

## Generation portability model

`GENERATION_ARCH_PORTABILITY_MODEL` = **CFDRT005 artifacts published per host from semantically identical source** — no cross-architecture generation blob interchange claim.

## Preserved nonblocking qualifications

| Item | Status |
|------|--------|
| Symlink TOCTOU | `BOUNDED_RESIDUAL_PRESERVED` (Phase 8 not authorized) |
| pread fallback runtime | `NOT_MEASURED_WITH_REASON` |
| EAGAIN / partial write | `SOURCE_PROVEN_PRESERVED` |
| FIFO non-regular probe | Bounded timeout + dataplane restart in harness; server health restored — not classified as product defect |

- Three-handler CFDRT005 **publish artifact** parse: **PASS** (`routes=4`, static+proxy+fcgi in routes file)
- Three-handler **HTTP runtime** on ARM64 (proxy/fcgi): **NOT_MEASURED** — bounded per owner §27; not a P7R-B claim

## Specialist audits (run `20260901T171500Z`)

| Agent | Verdict |
|-------|---------|
| [Vertex architect](d7d0e3bb-db4b-4e65-80b6-0bf353965018) | **ACCEPT** — ADR-044 preserved; no ARM64 fork; sendfile on StaticSend |
| [Security engineer](e528a7ff-3b05-4721-8a7d-d0359b077346) | **CONDITIONAL** — zero probe leaks; TOCTOU bounded residual preserved |
| [Research engineer](0ef56241-db79-4076-b02a-38a3b28ca689) | **PASS** — Linux ARM64 syscalls align with `static_serve.rs` |
| [GRC engineer](8631b16f-ca4e-4405-ba6f-48c4f716ef6c) | **PASS** — parent terminals preserved; invalid runs excluded |
| [Logic auditor](4605dec6-4c9b-40c3-86ef-bdffc747966e) | **PASS** — `STATIC_DUAL_ARCH_FALSIFIED=NO`; harness oracle P2/P3 only |

**Logic audit follow-up:** `OPEN_LOGIC_P2=3`, `OPEN_LOGIC_P3=4` are harness-oracle gaps on run `171500Z` (hardcoded three-handler PASS, debug-vs-release test split, inert FD gate). **No product MUST_FIX.** Harness repaired post-terminal for future runs; **no Oracle re-run** under `STOP_AFTER_TERMINAL=YES`. See evidence `LEDGER/logic_audit_followup.txt`.

## Invalid / superseded runs

Documented in evidence `LEDGER/invalid_runs.txt` — harness bugs only (digest path compare, FIFO hang, keepalive parser); **no product mutation**.

## Harness (evidence-only)

| Script | Role |
|--------|------|
| `scripts/reality/run-static-dual-arch-qualification-oracle.sh` | Native ARM64 matrix on Oracle |
| `scripts/remote/v044-static-dual-arch-qualification-oracle.sh` | rsync + remote run + evidence pull |

## Gates

- CFD-scoped `cargo check/clippy/test` on `exyonq-cfd-{gen,control,dataplane}`: **PASS** (Darwin local iteration + Oracle native tests in harness).
- Full workspace gates: **QUALIFIED** — large ambient dirty tree; attribution bounded to CFD packages (same posture as P7R-B).

## Static architecture scope

`STATIC_ARCHITECTURE_SCOPE` = **LINUX_AMD64_AND_LINUX_ARM64_VERIFIED** (Static handler portability only — not universal all-platform claim).

## Migration value

**YES (PARTIAL)** — native ARM64 Static parity increases confidence for Graviton/Ampere-style ARM cloud and mixed-arch fleets deploying CFD Static alongside existing AMD64 evidence; does not by itself prove full three-handler production topology on ARM64 (P7R-B remains Netcup amd64-only).

## Recommended next WIP (NOT authorized)

At STATICDUAL-B seal: `PHASE8_COMPLETENESS_AND_ADVERSARIAL_SECURITY` or `PRODUCTION_APP_THREE_HANDLER_TOPOLOGY_QUALIFICATION` — owner must authorize separately.

**Later event (does not rewrite CASE STATICDUAL-B):** Phase 8 was owner-authorized 2026-09-01 and terminated **P8-C** (`P8-STATIC-FIFO-001` Linux `openat2` FIFO hang). See [v044-phase8-completeness-and-adversarial-security.md](v044-phase8-completeness-and-adversarial-security.md). FIFO “harness restart” wording above remains the **historical STATICDUAL-B** classification.

## Related terminals (immutable)

- [v044-cfd-native-static-filesystem-handler-implementation.md](v044-cfd-native-static-filesystem-handler-implementation.md) — **P7STATICIMPL-B**
- [v044-phase7-static-proxy-fastcgi-requalification.md](v044-phase7-static-proxy-fastcgi-requalification.md) — **P7R-B**
