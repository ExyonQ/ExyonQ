# V044 — CFD native static filesystem handler implementation

```text
WIP = V044_CFD_NATIVE_STATIC_FILESYSTEM_HANDLER_IMPLEMENTATION
PARENT_TERMINAL = P7STATICADM-B
BINDING_ADR = docs/adr/044-cfd-native-static-filesystem-handler.md
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
TERMINAL_EVIDENCE_RUN = 20260901T005500Z
EVIDENCE = .exyonq-local/evidence/cfd-native-static-filesystem-handler-implementation/20260901T005500Z/
P7_REQUALIFICATION = NOT_AUTHORIZED (this WIP is handler-only bounded Netcup proof)
```

## Summary

Implemented **Accept A** from ADR-044: sync CFD-native `BackendKind::Static` with `CompiledStaticPolicy`, **CFDRT005** wire layout, native filesystem open/serve in `exyonq-cfd-dataplane` (`static_serve.rs` + `Phase::StaticSend`), Cap004-class containment (Linux `openat2` + canonicalize fallback), sendfile-preferred transfer with bounded pread fallback, and generation reload. Static miss is terminal **404/403** — no Proxy/FastCGI fallthrough.

**Terminal case:** `P7STATICIMPL-B` — core native Static is real and safe on Netcup amd64; nonblocking qualifications: symlink TOCTOU **BOUNDED_RESIDUAL** (Phase 8), Linux pread fallback path not runtime-proven, EAGAIN/partial sendfile not adversarially exercised on Netcup, single-arch evidence only.

**Dual-arch follow-on (separate WIP, does not rewrite this terminal):** [v044-static-dual-arch-qualification.md](v044-static-dual-arch-qualification.md) — **STATICDUAL-B** closes native ARM64 Oracle Static portability; `STATIC_ARCHITECTURE_SCOPE=LINUX_AMD64_AND_LINUX_ARM64_VERIFIED` for Static handler only.

## Product surfaces

| Surface | Path |
| --- | --- |
| IR / CFDRT005 | `crates/exyonq-cfd-gen/src/route_table.rs` |
| Control publish text | `crates/exyonq-cfd-control/src/project.rs` |
| Open/containment/MIME/IO | `crates/exyonq-cfd-dataplane/src/static_serve.rs` |
| Dispatch + StaticSend FSM | `crates/exyonq-cfd-dataplane/src/shard.rs` |
| Integration tests | `crates/exyonq-cfd-dataplane/tests/phase7_static_native.rs` |
| Netcup reality harness | `scripts/reality/run-cfd-native-static-filesystem-handler-netcup.sh` |

## CFDRT005 (Static policy wire)

- Magic: `CFDRT005` (encode always v5; decode accepts CFDRT002–005).
- `BackendKind::Static = 3`.
- `CompiledStaticPolicy`: `id`, absolute `document_root`, `flags` (must be 0), optional `index` basenames.
- Validation: absolute root, NUL forbidden, unknown flags rejected, static index rejects dotfiles and `.php` (case-insensitive), static routes must reference known policy id.
- CFDRT004 decode: empty `static_policies`; Static backend kind on legacy v4 payload rejected.

## StaticSend FSM

```text
open_static → static_response_head in to_client_head
  → HEAD only: WriteClientResponse → complete/close
  → GET body: Phase::StaticSend { file, offset, remaining, close }
      → write_pending_head_then_body
      → sendfile (Linux) or pread fallback
      → Pending on EAGAIN (park EPOLLOUT)
      → Complete → keepalive reset or Close
Errors before commit: queue_error (404/403/405/500) — no StaticSend.
Errors after commit: connection Close — no redispatch.
```

## Security (MVP)

- Root-only mapping; no alias/autoindex.
- URI segment deny: `..`, dotfiles, `.php` (case-normalized).
- Linux: `openat2(RESOLVE_BENEATH|RESOLVE_NO_MAGICLINKS)`; fallback Cap004 canonicalize + containment.
- Opened-FD identity; REGULAR_FILES_ONLY.
- Out-root symlink → 403 (EXDEV mapped); TOCTOU **not closed** (admitted residual).

## Reality evidence (Netcup amd64)

Authoritative run: `20260901T005500Z` — `RESULTS.env` `CORE_MATRIX=PASS`, `SENDFILE_REALITY=PASS_count_2`, content SHA oracles, FD/RSS sanity, sequential keepalive.

Binary: `b43781b1fb06a2c003c7be9b6ee1095b19483af553fb17e88a4be4a5d019b0ce` (linux-amd64 release `exyonq-dataplane`).

## Gates (CFD-scoped; ambient worktree)

- `cargo check/clippy/test` on `exyonq-cfd-gen`, `exyonq-cfd-control`, `exyonq-cfd-dataplane`: PASS (Darwin local iteration).
- Proxy regression: `phase2_h1_matrix` serial 9/9 PASS.
- FastCGI regression: `phase6b_fcgi` 5/5 PASS.
- Full workspace gates: **QUALIFIED** — large ambient dirty tree; attribution limited to CFD packages above.

## Specialist audits (implementation HEAD)

| Agent | Verdict |
| --- | --- |
| Vertex | PASS |
| Security | CONDITIONAL (TOCTOU residual; MUST_FIX=0) |
| Research | CONDITIONAL (primary sendfile proven; fallback/EAGAIN not runtime-proven) |
| GRC | CONDITIONAL (this binder closes doc gap; no P7/Phase 8 authorization) |
| Logic | PASS (LA-STATIC-001 P2 fixed: gen rejects `.php` static index) |

## Explicit non-goals (this WIP)

Phase 7 three-handler requalification, Phase 8, Range/ETag/cache/FD reuse, competitive benchmarks, merge/push/tag/release.

## Recommended next WIP (NOT authorized)

`V044_PHASE7_STATIC_PROXY_FASTCGI_REQUALIFICATION` — owner must authorize separately.

**Later event (does not rewrite CASE P7STATICIMPL-B):** Phase 8 terminated **P8-C** with `P8-STATIC-FIFO-001`. TOCTOU still not `CLOSED_STRONG`. See [v044-phase8-completeness-and-adversarial-security.md](v044-phase8-completeness-and-adversarial-security.md).
