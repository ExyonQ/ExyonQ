# Architecture — Competitive Frontier Phase 4: request bodies + bounded backpressure

```text
WIP = V044_COMPETITIVE_FRONTIER_REQUEST_BODIES_BOUNDED_BACKPRESSURE
PHASE = 4
CASE = (terminal)
PRODUCT_MUTATION = YES_BOUNDED_PHASE_4_ONLY
PHASE_5_AUTHORIZED = NO
```

## Primary question

Can ExyonQ support real HTTP/1 request and response body streaming with correct framing, bounded backpressure, disconnect handling, and smuggling defenses without destroying the competitive frontier or the split-plane architecture?

## Freezes (held)

Separate-process dataplane · mio/std · Tokio/Hyper = 0 · no per-request control RPC · Cap067 unchanged · shard-local pool · client+upstream same shard.

## Design (ADR-021)

Pattern **A**: fixed 16 KiB directional per-connection buffers. Whole-body buffering rejected. Request/response **chunked = fail-closed** this WIP (CL streaming required).

```text
MAX_BODY_BUFFER_BYTES_PER_CONNECTION = 32768
CL_TE_POLICY = REJECT_AND_CLOSE
DUPLICATE_CL_POLICY = REJECT_AND_CLOSE
FRAMING_DECISION_SSOT = http_parse → shard relay
```

## Body WAF

Product body rules exist; CFD projection **fail-closed** for body targets (`DEFERRED`). Header WAF deny ⇒ **0** upstream body bytes.

## Evidence

`.exyonq-local/evidence/competitive-frontier-request-bodies-backpressure/20260828-224420/`

## Header acquisition contract (hygiene close)

```text
CLIENT_HEADER_TIMEOUT = 5s absolute monotonic Instant budget
REFRESH_ON_DRIP = FORBIDDEN
IDLE_REFRESH_WHILE_READING_HEADERS = FORBIDDEN
```

Once `Phase::ReadClientHeaders { since }` is entered, `since` is preserved until
headers complete or the absolute deadline fires (`408` + close). Keepalive
`Idle → ReadClientHeaders` starts a new `since`. See governance
`v044-phase4-hygiene-residual-p2-close.md` (L4-P2-001).
