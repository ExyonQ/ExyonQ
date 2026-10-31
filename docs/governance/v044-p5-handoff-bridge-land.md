# Cap067 P5 handoff KEEP — land WouldBlock→Hyper + Tokio queue (2026-09-23)

```text
STATUS = KEEP
WIP = CAP067_P5_HANDOFF_BRIDGE
MEASUREMENT_CLASS = DEVELOPMENT
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
HOST = netcup-bench nodelayab
SUITE = p5-handoff-remasure-20260923T150839Z
```

## Defect on `release/v0.4.4-integration` before this land

1. `connection_executor::new` called `ensure_handoff_bridge` but `hyper_handoff.rs` on HEAD
   had no bridge → **Linux compile break** / silent missing symbol if somehow linked.
2. `should_handoff_on_accept` returned `Ok(false)` on `WouldBlock` → Cap067 StayAttached
   for unread P5 sockets → map fill → ~2–7k RPS (KEEP evidence 2026-09-20).

## Mutation landed

| Piece | Behavior |
| --- | --- |
| Cap067→Tokio handoff bridge | OS-thread handoffs enqueue; Tokio consumers `spawn` (not `Handle::spawn` from Cap067) |
| WouldBlock on accept peek | `force_hyper` / HyperReady |
| EPOLLET listen | drain while `accept_batch` returns full `ACCEPT_BATCH_LIMIT` |
| epoll keepalive divert fail | `Err(stream)` never `Ok(())` (LA-CAP067-OWN-001) |

## Remasure gate (DEV)

`scripts/remote/p5-handoff-remasure-netcup.sh`

- B = `EPOLL_LISTEN=1`, A = `EPOLL_LISTEN=0`, wrk `-t2 -c100 -d12s` Connection:close `/api/`
- PASS: B median ≥ 15k and B > HAProxy

## Netcup remasure 20260923T150839Z (wrk -t2 -c100 -d12s Connection:close `/api/`)

| Arm | median RPS | runs |
| --- | ---: | --- |
| B `EPOLL_LISTEN=1` | **43126** | 45302 / 41018 / 43126 |
| A `EPOLL_LISTEN=0` | 41243 | 41243 / 42778 / 39017 |
| HAProxy | ~10k med (noisy; r1 28665) | 28665 / 10256 / 2608 |

**GATE = PASS** (B ≥ 15k and B > HAProxy). B ≈ A (no Cap067 P5 collapse).

Prior local KEEP note: `.exyonq-local/evidence/p5-handoff-KEEP.txt` (B ~28k, A ~33k, 2026-09-20).
