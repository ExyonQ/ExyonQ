# P14V042-SCOPE-PROPOSAL — ExyonQ v0.4.2 (brief)

```text
DOCUMENT_KIND = SCOPE_PROPOSAL_ONLY
P14V042_OPENED = NO
P14V041_STATUS = COMPLETE
P14V041_CLOSE_STATUS = COMPLETE
BASELINE_RELEASE = v0.4.1
BASELINE_TAG_TARGET = 43805eb04a79babfbe443e2db4ca0d5b6658c80c
POST_TAG_LEDGER = 25dcb4e (docs only; do not retarget v0.4.1)
PUBLICATION_DEFAULT = FORBIDDEN_UNTIL_OWNER_AUTHORIZE
PUBLIC_OPENING = OUT_OF_SCOPE (remains 0.4.5 / 0.5.0 track)
LATEST = DO_NOT_TOUCH
```

Purpose: separate candidate work for a possible **private** `v0.4.2` after `P14V041` closure. This document does **not** authorize implementation, dependency bumps, signing, or publication.

---

## 1. Dependencies still pending

| Item | Status after v0.4.1 | v0.4.2 posture (proposal) |
|------|---------------------|---------------------------|
| Transitive `socket2` **0.5.10** via `redis` | Direct product use is on **0.6.5**; dual version remains (DBEX-006 residual) | **Policy review first** — eliminate only with redis upgrade path / containment plan; not a blind Dependabot unify |
| `serde_json` **1.0.150** | Left intentionally (no advisory/coupling in 0.4.1) | Reassess only if serde group or advisory pulls it |
| `notify` **9.0.0-rc.*** | Explicitly forbidden in 0.4.1 (prerelease + MSRV) | Keep **out** until stable + MSRV policy allows |
| Wasmtime / quinn / rustls / io-uring / matchit / memmap2 / criterion Dependabot branches | Not admitted in 0.4.1 | Admit **only** with advisory, active-use proof, or owner scope approve |
| Routine patch bumps (anyhow/serde/bytes line) | Matured for 0.4.1 | Scan at open time; no global `cargo update` |

---

## 2. Demonstrated functional defects / honesty gaps

| Finding | Evidence | Candidate for 0.4.2? |
|---------|----------|----------------------|
| OCI image `exyonq --version` reports `source_revision=unknown` | Netcup/Oracle digest smoke on `ghcr.io/exyonq/exyonq@sha256:ac0d74f1…` | **Yes (small)** — wire build-time revision into binary metadata (or document that only OCI label carries revision) |
| Config watcher thrash on parent `/tmp` | Fixed in 0.4.1 (`path-filter`) | **Closed** — regression guard only if touched |
| CHANGELOG still says 0.4.1 “not published” / `PUBLICATION_STATUS=FORBIDDEN` in places | Stale relative to private close | **Docs honesty** — eligible as docs-only when 0.4.2 opens |
| Ambient lab dirt / parallel perf branches | Process risk, not product defect | Process gate: clean worktree from `v0.4.1` / `43805eb` (or later authorized tip) |

No new blocking functional defect was left open that required retagging `v0.4.1`.

---

## 3. Security

| Item | Disposition |
|------|-------------|
| `RUSTSEC-2025-0134` `rustls-pemfile` unmaintained | Tracked in 0.4.1; **migration to `rustls-pki-types`** proposed for 0.4.2+ unless severity rises earlier |
| `EXYONQ-SEC-PRIVATE-MATERIAL-ZERO` | Keep fail-closed; no private TLS fixtures |
| Actions SHA-pin retention | Preserve; any new Action = SHA-pin required |
| Supply-chain (`cargo audit` / `cargo deny`) | Re-run at open; fix Critical/High; Medium with waiver policy |
| Public opening / visibility / `latest` | **Out of scope** for 0.4.2 |

---

## 4. Performance

| Item | Disposition |
|------|-------------|
| 0.4.1 dual-arch interleaved A/B | No durable >3% median product regression attributed to maturation |
| New perf work in 0.4.2 | **Only** with hypothesis + Netcup/Oracle evidence; no speculative hot-path refactors |
| Protectors | If shared paths change: P2–P13 no-regression discipline |
| Mac / Docker Desktop | `LOCAL_ITERATION_ONLY` / `NOT_LINUX_EVIDENCE` |

---

## 5. Expressly deferred from v0.4.1 (stay deferred unless re-authorized)

```text
- WAF complete; Nexus Panel; public admin API / OpenAPI; multinode
- Major HTTP/3 redesign; global QUIC provider substitution
- Full .htaccess compatibility; WordPress / cache enterprise
- Kernel redesign; intergenerational FastCGI pool reuse
- Speculative abstractions; unprofiled optimization
- Public repository opening; GHCR/GitHub visibility change; latest create/update/delete
- DBEX-006 full socket2 dual-version elimination (without redis strategy)
- notify 9.0.0-rc.*
- Reopening P14V040 / P14HISTCLEAN / P14TLSFIX / P14V041 close/sign/OCI/release
```

---

## 6. Suggested 0.4.2 shape (if owner later opens)

```text
PRIMARY_CATEGORY = private patch / honesty + security debt reduction
CANDIDATE_MIN =
  - docs honesty (changelog / publication status vs private v0.4.1)
  - rustls-pemfile migration plan or waiver refresh
  - OCI/binary source_revision wiring (small, measurable)
CANDIDATE_CONDITIONAL =
  - redis + transitive socket2 0.5 elimination under DB1 review
  - selective serde_json / other active-use bumps with evidence
EXPLICIT_NON_GOALS =
  - public opening; latest; force-push; history rewrite
  - product roadmap surfaces listed in §5
```

Admission still requires owner authorize (`P14V042_SCOPE` / implementation phase). Do **not** treat this proposal as authorization.

---

## 7. Recommended next step

1. Owner reviews this proposal.  
2. If accepted: open **`P14V042-SCOPE`** audit (not implementation) from clean `v0.4.1` line.  
3. Only after scope approve: open implementation with `PUBLICATION_STATUS=FORBIDDEN` until a later close authorize.
