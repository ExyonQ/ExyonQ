# V044 Competitive Frontier dataplane foundations (Phase 1)

**WIP:** `V044_COMPETITIVE_FRONTIER_DATAPLANE_FOUNDATIONS`  
**Phase:** 1 — foundations only (STOP; Phase 2 not authorized)  
**Parent:** CFR-B (`d94856663f42ffce16a3c0ccc1291feb4d7083e4`)

## Owner-frozen architecture

| Decision | Value |
|----------|-------|
| Process model | `HYBRID` — Tokio control/product + separate OS process for competitive H1 |
| Dataplane isolation | `SEPARATE_PROCESS` |
| Competitive H1 accept owner | Dataplane process only |
| Dual accept | Forbidden |
| Per-request control involvement | 0 |
| Tokio in dataplane | Absent |
| Hyper fallback on competitive H1 | Forbidden |
| Cap067 | Preserved (no rewrite) |
| SDP-P1 / PXDP execution | Not resurrected |

## Packaging

- Product CLI: `exyonq`
- Dataplane binary: `exyonq-dataplane` (crate `exyonq-cfd-dataplane`)
- Same product; not a separately branded project
- Feature gate (default OFF): `EXYONQ_COMPETITIVE_H1_DATAPLANE=1`
- Required when ON: `EXYONQ_CFD_LISTEN`, `EXYONQ_CFD_GEN_DIR`, optional `EXYONQ_CFD_SHARDS`, `EXYONQ_DATAPLANE_BIN`
- Process exclusivity: advisory `flock` on `$GEN_DIR/dataplane.lock` (prevents second REUSEPORT owner)
- READY is published only after all shards successfully bind + register with the poller

## Crates

| Crate | Role |
|-------|------|
| `exyonq-cfd-gen` | Immutable generation encode/decode, CRC32, atomic tmp→rename publish |
| `exyonq-cfd-control` | Launch/lifecycle, listen-collision check, parallelism budget |
| `exyonq-cfd-dataplane` | mio shard loops, exclusive listen (SO_REUSEPORT among DP shards only) |

## Generation publication (ADR-021)

Selected: **atomic file swap + read-only load** (`generation.bin` under a 0700 directory).

Semantics: G1 active → publish G2 → new work observes G2 → downgrade (lower id) rejected → corrupt/CRC/schema rejected.

Not used per request: RPC, socket handoff, policy lookup to control.

## Listener ownership

Dataplane shards bind the competitive H1 address with `SO_REUSEPORT` **among dataplane shards only**. Control process must not share that port: CLI rejects listen collision with product `[[server]]` / HTTP3 listens when the gate is ON.

## Shard model

- Connection owned by accepting shard for lifetime
- No cross-shard handoff
- Shard count: explicit override or CPU-derived budget (not hard-coded-only-4); clamp 1..=128

## Parallelism budget

`ExyonqParallelismBudget` reserves competitive H1 shards without rewriting Cap067. Documentary control/static/fastcgi/blocking fields exist for later coordination.

## Phase-1 traffic surface

- `GET /__exyonq_cfd/v1/foundation` → 200 + generation header (qualification probe)
- Any other path → **503** `X-Exyonq-Cfd-Unfinished: 1` (no Hyper, no product routing)

## Out of scope (Phase 1)

Routing, WAF, bodies, FastCGI migration, Cap067 rewrite, H2/H3 rearchitecture, Windows, full P1–P14 matrix, SDP-P1/PXDP resurrection.

## Phase-2 gate (owner review required)

Real H1 GET + routing + headers — **not** auto-started from CFD-A.
