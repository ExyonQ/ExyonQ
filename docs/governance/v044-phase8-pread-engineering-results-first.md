# V044 Phase 8 — pread fallback engineering (results first)

```text
WIP = V044_PHASE8_PREAD_ENGINEERING_RESULTS_FIRST
CASE = PREAD-A
STATUS = PREAD_FALLBACK_RUNTIME_PROVEN_DUAL_LINUX
PHASE_8_CLOSED = YES (via P8DOCSEAL-A / v044-phase8-canonical-documentation-seal.md)
OWNER_AUTHORIZATION = YES_ENGINEERING_AND_RUNTIME_EVIDENCE
```

## Summary

Linux static TCP body path in `exyonq-cfd-dataplane` uses `sendfile` with fallback to `read_at` (pread) on **EINVAL** or **ENOSYS** only, reusing the same confined FD (`static_serve.rs`).

| Check | Result |
|-------|--------|
| Natural trigger (Netcup amd64 / Oracle arm64) | **Not observed** — regular file→pipe sendfile succeeds |
| Controlled oracle (`test-utils`) | **20/20** integration reps per arch + strace `pread64` on payload |
| Normal sendfile control | **5/5** per arch |
| Ineligible errno negative control | **PASS** per arch |
| Product defect | **Not proven** |
| Production mutation | **No** (test-utils seam not in release builds) |

## Code audit

| Field | Value |
|-------|-------|
| Sendfile | `linux_sendfile` → `send_static_body_sendfile` |
| Pread fallback | `send_static_body_pread` via `file.read_at()` |
| Eligible errnos | `EINVAL`, `ENOSYS` |
| Same-FD guarantee | Yes — no reopen-by-path on fallback |

## Commits (candidate worktree)

| Commit | Summary |
|--------|---------|
| `03518a75` | test-utils sendfile errno seam + oracles + harness |
| `aa2398f0` | strace/oracle fixes; ineligible control semantics |
| `c82bbcdc` | strace `-f`; unit-test syscall capture |
| `57647b2b` | pread attribution via payload in strace |
| `6f713ead` | grep prefix for strace truncation |
| `<doc_bind>` | seal docs + terminal block |

Entry: `878e576c` (post PREAD-D abort docs). Functional base: `82ac4ec2` (FCGITD-A).

## Authoritative evidence

```text
RUN_ID = 20260903T143458Z-seal
SOURCE_HEAD = 6f713eadbc443285c94dcbff2d28cbbdc5e3fd82
EVIDENCE = .exyonq-local/evidence/phase8-pread-engineering-results-first/20260903T143458Z-seal/
```

Both arches: `PREAD_ORACLE=PASS`, `PREAD_FALLBACK_PASS=20_OF_20`, `PREAD_PAYLOAD_ATTRIBUTION=80`.

Payload SHA256: `75294c5035990ca0560325707004aa9391928303d79b95cc9b7c0190b8ff2b06`.

## Limitations

- Controlled errno is injected at the sendfile wrapper (may not appear as `sendfile` in strace); `pread64` on oracle payload is the syscall attribution oracle.
- Natural EINVAL/ENOSYS on supported Linux static→TCP was not reproduced without the test seam.
- Phase 8 is closed canonically via **P8DOCSEAL-A** (`v044-phase8-canonical-documentation-seal.md`). Phase 9 / official competitive benchmarks remain **not authorized**.

Terminal block: `docs/governance/v044-phase8-pread-engineering-results-first-terminal-block.txt`
