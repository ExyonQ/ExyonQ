# Architecture — Competitive Frontier Phase 3: WAF + generation reload

```text
WIP = V044_COMPETITIVE_FRONTIER_WAF_GENERATION_RELOAD
PHASE = 3
CASE = CFW3-B
PRODUCT_MUTATION = YES_BOUNDED_PHASE_3_ONLY
PHASE_4_AUTHORIZED = NO
PHASE_4_AUTOSTART = NO
```

## Primary question

Can ExyonQ add **real** WAF + generation/reload on the competitive frontier dataplane (CFD) without destroying the Phase-2 competitive frontier vs rivals?

**Answer (bounded):** Yes for frontier retention vs nginx/OLS/HAProxy on Netcup AMD64 with a nontrivial WAF ruleset. Incremental A0(WAF-off)→A1(WAF-on) cost is **PARTIAL** (not Latin-square isolated; see performance budget).

## Freezes (held)

| Freeze | Status |
|--------|--------|
| Separate-process `exyonq-dataplane` | held |
| mio/std threads; Tokio/Hyper in dataplane = 0 | held |
| No per-request control RPC | held |
| Cap067 unchanged | held |
| Bodies / Phase 4 | not started |

## Geometry

```text
CONTROL = exyonq-cfd-control (SSOT compile_waf → CFDWAF01 + CFDCOM01 publish)
DATAPLANE = FixedWafEngine from projection; inspect before route/upstream
GENERATION = GenDir monotonic live_gen; shard local apply; corrupt keep-prior
CLIENT_IP = TCP peer only (no XFF trust)
BODY_RULES = NOT_SUPPORTED_IN_PHASE3 (fail at project)
```

Request order:

```text
parse → WAF → route → hop-by-hop → upstream
```

Deny ⇒ no upstream write (Darwin hit-counter E2E + Netcup 403 `waf denied`).

## Wiring

- `exyonq-cfd-gen`: `waf_proj` / `composite` (`CFDWAF01`, `CFDCOM01`; legacy `CFDRT002`)
- `exyonq-cfd-control`: `projection_from_compile_input`, `representative_phase3_waf_input`, `publish_composite`
- `exyonq-cfd-dataplane` `shard.rs`: WAF before route

## Evidence

`.exyonq-local/evidence/competitive-frontier-waf-generation-reload/20260828-212500/`
Binary SHA256: `a0d79d8f3f7aeea03b7d53b2a6e25e7d6802d83912a197a9caf1653e74d2184f`

## Phase 4

Not authorized. Stop after CFW3 terminal.
