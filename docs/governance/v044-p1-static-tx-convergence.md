# V044 P1 static-TX path convergence

**Status:** product fix landed (`f2e90beb`) — **functional** sendfile convergence ACCEPTED.
RPS attribution of static-TX as primary competitive root is **NOT_CONFIRMED** (post-fix RPS did not close the gap).
SessionTable mutex later fixed (`427b1739`) and causally confirmed — see `docs/governance/v044-p1-cap067-sessiontable-lock.md`.
Current residual (post-SessionTable): `docs/governance/v044-p1-post-sessiontable-residual.md`.
Pre-SessionTable residual archive: `docs/governance/v044-p1-residual-gap-reprofile.md`.

## Problem

Canonical symmetric-Docker P1 load uses rewrk with an absolute-form HTTP/1.1 request-target:

```http
GET http://exyonq:8080/site/1k.bin HTTP/1.1
```

Cap067 epoll sendfile eligibility parsed the **raw** request-target and required a leading `/`.
Absolute-form targets therefore failed `epoll_sendfile_eligible` and fell through to Hyper static
materialization (`openat` + userspace read + `sendto`/`write`), despite an existing verified
sendfile engine.

Origin-form clients (`GET /site/1k.bin`) already used sendfile.

## Fix (production, not benchmark-only)

In `crates/exyonq-mod-static/src/wire_eligibility.rs`:

- Extract **origin path** from origin-form and `http(s)://authority/path` absolute-form targets (RFC 9112).
- Apply the same path to sendfile eligibility and `match_sendfile_asset` via `request_path_from_head`.
- Reject `*`, authority-form, and unknown schemes.
- Exclude `/health`, `/metrics`, and `/api/*` after extraction.

No P1 filename special case, no second sendfile engine, no loadgen detection branch.

## Eligibility contract (unchanged semantics, corrected parsing)

| Request-target | Sendfile candidate |
| --- | --- |
| `GET /site/1k.bin` | yes (static GET/HEAD, Linux Cap067 auto-on) |
| `GET http://host/site/1k.bin` | yes (same origin path) |
| `GET http://host/api/x` | no |
| `GET http://host/health` | no (ops probe) |
| `GET * HTTP/1.1` | no |

## Fallback

Ineligible resolve, open failure, 512-byte handoff overflow, or non-Linux → existing Hyper /
buffered static path (unchanged).

## Runtime proof (Netcup amd64)

Under rewrk absolute-form load, strace shows `sendto(..., MSG_MORE)` + `sendfile()` per response
(evidence: `.exyonq-local-evidence/v044-p1-static-tx-after/` on bench host).

## Residual notes

- FD reuse / openat caching: **not** implemented; residual reprofile marks `FD_REUSE_REQUIRED = NOT_PROVEN` (openat2/req≈1 remains, but Cap067 session mutex/futex dominates post-fix samples).
- Ops probes (`/health`, `/metrics`) remain origin-form-only for wire fast path (pre-existing).
- Do **not** treat pre-fix `STATIC_TX_GAP_SHARE = 40–55%` as current causal truth.
- Post-fix evidence pack: `.exyonq-local/benchmarks/v044-p1-residual-reprofile-20260820T020434Z/`.
