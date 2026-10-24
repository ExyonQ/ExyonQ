# Governance — V044 Competitive Frontier Phase 2 (Real H1 + Routing + Headers)

```text
WIP = V044_COMPETITIVE_FRONTIER_REAL_H1_ROUTING_HEADERS
PHASE = 2
STATUS = OWNER_AUTHORIZED
PARENT = V044_COMPETITIVE_FRONTIER_DATAPLANE_FOUNDATIONS / CFD-A
PRODUCT_MUTATION = YES_BOUNDED_PHASE_2_ONLY
PHASE_3_AUTHORIZED = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
NOTION_RECONCILIATION_REQUIRED = YES
```

## Freezes (binding)

- All Phase-1 CFD-A freezes remain in force
- `HYPER_FALLBACK_ON_COMPETITIVE_H1 = FORBIDDEN`
- `WAF_IMPLEMENTED = NO`
- `REQUEST_BODY_IMPLEMENTED = NO`
- `CAP067_CHANGED = NO`
- `OPEN_P3` carried: advisory flock PID text residual (nonblocking; unrelated to H1 path)

## Integrity

- No smoke as terminal evidence
- No mock upstream as terminal evidence
- No hardcoded benchmark routes
- Real TCP upstream for E2E qualification
- Netcup AMD64 required for material Phase-2 performance disposition
- Darwin allowed for correctness iteration only

## Notion delta

1. Phase 1 CFD-A closed → Phase 2 authorized.  
2. Phase 2 adds real GET + Cap033 projection + hop-by-hop + KA pools.  
3. Phase 3 WAF still unauthorized.  
4. Link docs under `docs/architecture|governance|performance/v044-competitive-frontier-real-h1*`.

## Evidence root

`.exyonq-local/evidence/competitive-frontier-real-h1-routing-headers/<run-id>/`

## Terminal disposition (this WIP)

```text
CASE = CFH2-B
PERFORMANCE_BUDGET = YELLOW
LOGIC_AUDIT = PASS
PHASE_3_AUTHORIZED = NO
PRODUCT_COMMITS_CREATED = 0
```
