# ADR-021 pre-repair analysis — Upstream pool return (NOT IMPLEMENTED)

```text
STATUS = ANALYSIS_ONLY
PRODUCT_MUTATION = NO
IMPLEMENTATION_AUTHORIZED = NO
```

## Problem

CFD `UpstreamPool::put` is intentionally unreachable after Phase 4. Success path always `discard`s the upstream TCP stream. Client keepalive remains. Rivals typically reuse upstream connections → ExyonQ pays extra connect/handshake cost (anti-ExyonQ asymmetry).

## Root cause (verified)

Phase-4 fail-closed: non-blocking trailing-byte probe can race with peer writes; policy `UPSTREAM_TRAILING_BYTES = POISON` / no pool reuse after response.

## Ownership / mutability

| Object | Owner | Mutability |
|--------|-------|------------|
| Idle map | Shard-local `UpstreamPool` | `&mut` on shard thread only |
| TcpStream | Conn → pool or drop | move-only; no `Arc<Mutex<TcpStream>>` |
| `reuse_ok` | Computed at header parse | Currently ignored at finish |

## Lifecycle (target if re-enabled)

```text
take → write req → read resp → (if reuse_ok && framing complete && no trailing ambiguity)
  → put
else → discard
```

## Concurrency

Shard-local only; no cross-shard sharing. No new global locks.

## Error / generation / poison

- Connect/write/parse/timeout/EOF/client abort → discard (unchanged).
- Generation reload: pool entries must be gen-tagged or flushed on generation bump (missing today; required if put returns).
- Poison: trailing bytes, over-CL, ambiguous framing → discard.

## Hot-path / allocation

- Avoid `Arc`/`Mutex` on connection; keep move semantics.
- No clone of buffers for pool return.
- Idle TTL + max-per-target already sketched in `upstream_pool.rs`.

## Alternatives

| ID | Approach | Pros | Cons |
|----|----------|------|------|
| **A** | Restore direct `put` after complete known-length relay | Minimal diff; matches Phase-2 shape | Must solve trailing-byte race |
| **B** | RAII lease: drop returns only if `CleanComplete` typestate | Harder to forget put; explicit states | More types; careful cancel paths |
| **C** | State-machine disposition enum on Conn | Clear auditability | Larger shard refactor |
| **D** | Retire pool; document connect-per-request | Honest contract; less code | Migration optics; distant/TLS cost |

**NO_NEW_PATTERN preference:** prefer **A** or **B** over novel frameworks; **D** if security cannot accept idle reuse.

## Cleanup / rollback

Feature flag or generation-gated `put` with instant kill-switch to discard-only.

## Expected performance

```text
EXPECTED_PERFORMANCE_GAIN = UNKNOWN_UNTIL_MEASURED
```

Causal expectation only: fewer `connect` syscalls, less FD churn, less upstream accept pressure — **no RPS/CPU % claim**.

## Decision for this WIP

```text
IMPLEMENT = NO
NEXT = owner chooses optional future repair WIP or permanent contract demotion of C5
```
