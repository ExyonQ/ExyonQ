# ADR-058 — WebSocket / keepalive guarantees under reload & drain (Q-05 / v0.6 F2)

**Status:** Accepted  
**Date:** 2026-10-18  
**Depends on:** Cap031 tunnel hold, Cap040 drain admission, Cap052 control socket  
**Contract:** CONDITIONAL ACCEPT — document and prove what survives; no silent drops of live tunnels

## Context

QUEJAS_NGINX §2.4 / QUEJAS_OLS §2.3: operators fear reload/drain cutting Home Assistant / n8n WebSockets. Cap031 already extends `active_connections` for upgraded tunnels so drain cannot complete while tunnels live.

## Guarantees (product)

| Event | Live H1 WebSocket tunnels | New WebSocket upgrades | New HTTP (non-upgrade) |
|-------|---------------------------|------------------------|-------------------------|
| **Generation reload** (PREPARE→COMMIT) | **Survive** — tunnel tasks keep their accepted upstream; no forced close | Use **new** generation routing | Use new generation |
| **Drain** (`exyonqctl drain`) | **Survive** until client/upstream close; counted via Cap031 hold | **Rejected** (typically 503 + draining) | **Rejected** (503 EXY-RT-0006 path) |
| **Shutdown** | Best-effort; not a zero-loss promise | Rejected | Rejected |

### Explicit non-guarantees

- HTTP/2 Extended CONNECT WebSocket / HTTP/3 WebSocket — unsupported current contract.  
- Upstream process crash / TCP reset — application must reconnect.  
- Changing the **bound listen address** or process replace without supervisor — out of scope (restart).  
- Zero frame loss if the **client** or **upstream** closes during reload — only ExyonQ must not tear down the tunnel solely because of reload/drain.

### Keepalive (HTTP/1.1)

Established keepalive connections may finish in-flight requests after drain latch; new requests on the same connection are subject to per-request drain admission (Cap040). Not measured in the Q-05 WS suite.

## Proof

- Cap031 E2E: single-tunnel drain keep + reload coherence (`scripts/reality/cap-031-websocket-e2e.py`).  
- Q-05 E2E: **N=100** concurrent tunnels survive reload then drain without lost echo frames (`scripts/reality/q05-websocket-reload-drain-e2e.py`).

## Consequences

- Operators may reload under live WS load without expecting a mass disconnect.  
- Drain is for shedding **new** work while tunnels finish — not an instant kill of WS.  
