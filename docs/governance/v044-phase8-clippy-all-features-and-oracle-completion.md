# V044 Phase 8 clippy --all-features and oracle completion

```text
WIP = V044_PHASE8_CLIPPY_ALL_FEATURES_AND_ORACLE_COMPLETION
CASE = P8TECH-D
STATUS = CLIPPY_PASS_ORACLES_ADVANCED_PREAD_AND_TRAILING_STILL_INCOMPLETE
PARENT = P8RECON-A
OWNER_AUTHORIZATION = YES_BOUNDED_TECHNICAL_COMPLETION
PHASE_8_CLOSED = NO
```

## Exit taxonomy

```text
CASE = P8TECH-D
```

Not **P8TECH-A**: `STATIC_PREAD_FALLBACK_RUNTIME` remains `NOT_MEASURED` on supported Linux TCP (sendfile succeeds; EINVAL/ENOSYS→pread not forceable without a product-only hook).  
Not **P8TECH-B**: no new supported product defect proven in-scope for this WIP.  
`trailing_data_forces_discard_error` was an intermittent Linux/Darwin test residual at seal time; product defect not proven. Follow-on **FCGITD-A** closed it as a harness race (co-buffered peer); see [v044-phase8-fcgi-trailing-data-causal-closure.md](v044-phase8-fcgi-trailing-data-causal-closure.md).

## What closed

| Gate | Result |
| --- | --- |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | PASS (commit `42f045b9`) |
| `cargo fmt` / `cargo check --all-features` / `cargo test --workspace --all-targets --all-features` | PASS (Darwin) |
| FCGI pool `apply_timeouts…` 20-rep | 0/20 fail Darwin + Netcup + Oracle |
| Dual-Linux CFD native builds (2×/arch) | PASS; amd64 `eee3e4d9…` arm64 `72652758…` (match P8R-BR; CFD unchanged by clippy fix) |
| Phase 8 EAGAIN (strace sidecar) | PASS both arches (`EAGAIN_OBSERVED=YES`, full hash) |
| Phase 8 partial sendfile | PASS both arches (multi-sendfile + hash) |
| `FASTCGI_SCRIPT_MAPPING_CONFUSION` | 0 + NEG_CONTROL PASS |
| Anti-Kobayashi | `PASS_EVIDENCE_BOUND` (NC1/NC2/NC3) |

## What remains open

| Residual | Status |
| --- | --- |
| `STATIC_PREAD_FALLBACK_RUNTIME` | NOT_MEASURED (reachability documented; blocks P8TECH-A) |
| `GATE_PHASE6B_FCGI` / `trailing_data_forces_discard_error` | Intermittent Linux FAIL (amd64 12/20, arm64 9/20 in this WIP series); Darwin 0/20 |
| Parent P3 IDs | Still open unless separately closed (pread/TOCTOU hash/config disclosure accounting) |
| Phase 8 close / Phase 9 | Forbidden; not claimed |

## Product mutation (clippy only)

Commit `42f045b9`:

- `exyonq-mod-proxy`: cfg-gate progressive/SSE helpers unused under `p8-transport-diag`; remove unused `StreamingWireWriter` trait surface.
- `exyonq-mod-http3`: remove always-true `--all-features` `compile_error` for quinn-legacy + modern providers; quinn-legacy active only when exclusive.

```text
PRODUCT_BEHAVIOR_CHANGE_FOR_NORMAL_REQUESTS = NO
CFD_DATAPLANE_BINARY_SHA_UNCHANGED = YES
```

## Harness mutation

Commit `5d51a03c`:

- Import P8R-BR EAGAIN harness (`ba4e79db…`) into `scripts/reality/`.
- Phase 8: strace sidecar + 120s slow reader; SCRIPT_FILENAME prefix authority; Anti-Kobayashi negative controls; honest pread NOT_MEASURED reason.

## Evidence

```text
EVIDENCE = .exyonq-local/evidence/phase8-clippy-all-features-oracle-completion/20260902T192255Z/
```

Canonical fingerprints unchanged (`cd14277e` / ambient tracked+untracked freeze). Candidate worktree only.

## Explicit non-claims

```text
PHASE_8_CLOSED = NO
PHASE_9 = NOT_STARTED
CANONICAL_MERGE = NO
COMPETITIVE_BENCH = NO
P8TECH-A = NO
PRODUCT_DEFECT_PROVEN = NO
```
