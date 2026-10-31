# P1 peak recovery — Cap067 under EPOLL_LISTEN=1 (2026-09-23)

```text
STATUS = DESIGN_ADMIT
WIP = CAP067_P1_PEAK_RECOVERY
MEASUREMENT_CLASS = DEVELOPMENT
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
PRIOR_KEEP = p5-handoff (WouldBlock→HyperReady) — P5 ≈ epoll0; P1 regresses
```

## Problem

With `EPOLL_LISTEN=1` KEEP (WouldBlock→HyperReady), wrk often accepts before GET bytes
are buffered → Cap067 hands the conn to Hyper. Static Cap067 peak (~280–310k on
`EPOLL_LISTEN=0`) falls to ~140k. P5 stays healthy (~43k).

## Candidate mutations (pick one to A/B)

| ID | Idea | Risk |
| --- | --- | --- |
| **A** Dual `SO_REUSEPORT` | Tokio + Cap067 both listen; Cap067 peeks only when bytes ready; Tokio owns unread | Complexity; fair-share accept |
| **B** Hyper→Cap067 divert | After Hyper reads first line, divert static-eligible to Cap067 keepalive/sendfile | Mid-conn ownership (P4-LA-001 sticky care) |
| **C** Deferred peek | Cap067 StayAttached on WouldBlock but arm short timer / edge wait before Hyper | Re-risk P5 map fill if timer wrong |

## Admission answers

1. Problem: recover Cap067 P1 peak without re-breaking P5.
2. Metric: P1 med RPS (quiet Netcup); protector P4/P5 ≥ −2%.
3. Competitor: match/beat OLS static peak under same EPOLL_LISTEN=1 product default.
4. Outside core?: No — listen/accept ownership is core Cap067.
5. Complexity: A high; B medium; C medium — budget one candidate.
6. Kill switch: `EPOLL_LISTEN=0` or revert WouldBlock→true.

## Recommended first A/B

**B (divert)** scoped: only `GET` static-eligible after Hyper header read; never divert
`/api/`, `/metrics`, AE-preferring compression. Remasure P1×3 + P5×3 + P4×3.

## Non-goals

- Official publish
- Changing WouldBlock→HyperReady KEEP without dual protectors green
