# V044 P1 — Cap067 miss / NeedHyper hang repair

```text
STATUS = CLOSED_FIXED_AND_VERIFIED_REAL_PRODUCTION
CASE = H1
WIP = V044_P1_CAP067_MISS_NEEDHYPER_HANG_REPAIR
WIP_LIMIT = 1
PRODUCT_COMMIT = 570272985d9cbfe13ccb0910c097edd89d6fc256
PRODUCT_TREE = 1c97dd9483d9d4d1715dbf67864a1674bf8dc596
FD_REUSE_CAUSAL = NO
FD_GEOMETRY_ROOT = CLOSED
FD_REUSE_REPAIR_RESULT = PROVEN_MATERIAL_SUCCESS
LA_CAP067_FD_001_STATUS = REMAINS_FIXED
P2_STARTED = NO
USERSPACE_RESIDUAL_ATTRIBUTION_STARTED = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
```

## Authority baseline (pre-mutation)

```text
BASELINE_HEAD = daa6f8b93b4f4ea5845f0875219f60e60e2a5231
BASELINE_TREE = cdf4652d95cca55505fd682fc41b1033bcf6ab4d
```

Ambient unrelated dirt in the lab worktree was preserved (claim-evidence integrity, WAF, profiling scripts, etc.). This WIP mutated only Cap067 miss/NeedHyper hang surfaces.

## Defect

On Linux Cap067 (epoll sendfile), sendfile-eligible GET/HEAD with
`match_sendfile_asset == None` emitted `NeedHyper`. Observed hang: curl timeout,
0 response bytes — including paths never present in the FD-reuse cache.

```text
FD_REUSE_CAUSAL = NO
```

Do not reopen the FD-reuse hit-path design for this defect.

## Root cause (proven)

```text
ROOT_CAUSE =
  NeedHyper → handoff_to_hyper(prefetched)
  → WirePlan::Static → register_sendfile_from_tokio_first
  → Cap067 epoll again → match None → NeedHyper (ping-pong)

ROOT_CAUSE_CONFIDENCE = PROVEN
SAFE_HYPER_HANDOFF_POSSIBLE = NO  # for Cap067-owned Cap004/missing terminals
REQUEST_BYTES_ALREADY_CONSUMED_AT_HANDOFF = YES  # head+body gated before NeedHyper
```

`NeedHyper` has a real consumer (`handoff_to_hyper`), but for Cap067-owned static
sites the Hyper divert re-registers sendfile and re-enters the same miss class.
That is not a missing consumer; it is an invalid ownership transfer for owned misses.

## NeedHyper ownership contract (reconstructed)

```text
NEEDHYPER_PRODUCERS =
  epoll_worker::sendfile_read_and_serve (decline-only after this repair);
  other Cap067 ineligible / mechanism-off paths

NEEDHYPER_CONSUMERS =
  epoll_worker ReadServe::NeedHyper → handoff_to_hyper → wire_dispatch

NEEDHYPER_OWNERSHIP_CONTRACT =
  Transfer connection+prefetched head/rest to Hyper only when Cap067 declines
  ownership of the request. Cap067-owned Cap004/missing/open-fail terminals must
  complete in epoll (no Hyper Static re-loop).

NEEDHYPER_SOCKET_OWNER_BEFORE = epoll_worker ConnMap
NEEDHYPER_SOCKET_OWNER_AFTER  = Hyper (on genuine decline) | epoll continues (terminal miss)
NEEDHYPER_REQUEST_OWNER_BEFORE = epoll parse buffer (bytes may already be consumed)
NEEDHYPER_REQUEST_OWNER_AFTER  = Hyper prefetched Bytes OR epoll continues after miss wire
```

## Pattern selection (ADR-021)

```text
ALTERNATIVE_A = repair generic NeedHyper handoff so Hyper never re-enters Cap067
ALTERNATIVE_B = complete known Cap004/miss terminals in epoll; NeedHyper = decline only
NO_NEW_PATTERN_ALTERNATIVE = minimal local state-machine correction

PATTERN_SELECTED = ALTERNATIVE_B (existing Cap067 ownership; no new named pattern)
WHY_SELECTED =
  safe generic handoff after epoll consumption is not available without faking replay;
  Cap004 security decisions must not reopen via a less-restrictive Hyper static path;
  terminals already exist in static wire (404/403); epoll must own them.
REFACTORINGS_APPLIED = NONE (no FD-cache redesign; no perf opts)
```

## Classification of `match_sendfile_asset == None`

| Class | Product outcome (Cap067-owned) |
|-------|--------------------------------|
| NOT_FOUND / DELETED / open fail | HTTP 404 (GET body; HEAD headers-only) |
| PATH_TRAVERSAL | HTTP 403 |
| SYMLINK_CONTAINMENT_REJECTED | HTTP 403 (no secret disclosure) |
| NOT_SENDFILE_ELIGIBLE / mechanism off | NeedHyper (genuine decline) |
| TOCTOU open succeeds after match miss | Decline terminal → single divert may serve hit |

## Security

```text
CAP004_CONTAINMENT_PRESERVED = YES
RESOLVE_BENEATH_PRESERVED = YES
PATH_TRAVERSAL_REJECTION_PRESERVED = YES
SYMLINK_ESCAPE_REJECTION_PRESERVED = YES
NO_SECRET_DISCLOSURE = YES
```

## Product files

- `module-api/src/static_epoll.rs` — `sendfile_miss_http_wire` hook
- `crates/exyonq-mod-static/src/epoll_session.rs` — classify miss → wire
- `crates/exyonq-mod-static/src/wire.rs` — HEAD 404/403 wires
- `crates/exyonq-mod-static/src/kernel_hooks.rs` — install hook
- `crates/exyonq-platform-linux/src/epoll_worker.rs` — terminal miss / begin-reject 503
- `core/src/server/wire_dispatch.rs` — comment only (no Semantic change)

Harness:

- `scripts/reality/cap-067-miss-needhyper-real-e2e.py`
- `scripts/reality/cap-067-fd-reuse-real-e2e.py` — Cap048 `EXYONQ_CONFIG` for reload

## Dual-arch real E2E (post-repair)

```text
NETCUP_AMD64_CAP067_MISS_REAL_E2E = PASS
  binary_sha256 = 5702715e538d8b0607193638fb2488defed7b3385523fecfaae9491559801250
ORACLE_ARM64_CAP067_MISS_REAL_E2E = PASS
  binary_sha256 = 4b11c46cb7f8fec1693d5739a2071c9f3cdf7466175c18290eff22d78181867f

ARM64_GET_REUSE = PASS
ARM64_KEEPALIVE = PASS
ARM64_ATOMIC_RENAME = PASS
ARM64_DELETE_RECREATE = PASS
ARM64_RELOAD_GENERATION = PASS
ARM64_PATH_TRAVERSAL = PASS
ARM64_SYMLINK_CONTAINMENT = PASS
ARM64_FD_CLEANUP = PASS

ORACLE_ARM64_CAP067_REAL_E2E = PASS
ORACLE_ARM64_REAL_PRODUCTION = PASS_FOR_VERIFIED_CAP067_SCOPE

FD_REUSE_HIT_PATH_NONREGRESSION = PASS
OPENAT2_CALLS_PER_REQ_ON_HOT_HIT ≈ 0
NEWFSTATAT_CALLS_PER_REQ_ON_HOT_HIT ≈ 1
```

## Clippy

```text
CAP067_MISS_REPAIR_CLIPPY_D_WARNINGS = PASS  # scoped packages --no-deps -D warnings
FD_REUSE_AFFECTED_SCOPE_CLIPPY_D_WARNINGS = PASS
GLOBAL_CLIPPY_D_WARNINGS = NOT_PROVEN_DUE_TO_PREEXISTING_UNRELATED_STATE
  # module-api/src/static_wire.rs type_complexity — out of this WIP
```

## Logic audit

```text
LOGIC_AUDIT_AGENT = exyonq-logic-auditor
LOGIC_AUDIT_STATUS = PASS_WITH_NONBLOCKING_P3_FINDINGS
OPEN_LOGIC_P0 = 0
OPEN_LOGIC_P1 = 0
OPEN_LOGIC_P2 = 0
OPEN_LOGIC_P3 = 2  # TOCTOU Ok→None divert; Io→404 vs Hyper 500 opacity
LA-CAP067-MISS-001 = CLOSED  # HEAD bodyless
LA-CAP067-MISS-002 = CLOSED  # begin 503 HEAD bodyless
```

## Closeout relation to FD-reuse verification

```text
V044_P1_FD_REUSE_VERIFICATION_CLOSEOUT_STATUS = COMPLETE
OPEN_PROVEN_PRODUCT_DEFECTS = 0
CAP067_MISS_NEEDHYPER_HANG_STATUS = CLOSED_FIXED_AND_VERIFIED_REAL_PRODUCTION
```

## Post-repair P1 seal (read-only rebaseline)

```text
SEE = docs/governance/v044-p1-final-post-repair-baseline.md
P1_FINAL_STATUS = CLOSED_CURRENT_COMPETITIVE_BASELINE
CASE = S1
PRODUCT_MUTATION = NO
P2_STARTED = NO
```
