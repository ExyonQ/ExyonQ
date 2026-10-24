# Governance — V044 Competitive Frontier dataplane foundations

```text
WIP = V044_COMPETITIVE_FRONTIER_DATAPLANE_FOUNDATIONS
PHASE = 1
STATUS = OWNER_AUTHORIZED
PARENT = V044_CORE_ARCHITECTURE_RECONCILIATION_FOR_COMPETITIVE_FRONTIER / CFR-B
PRODUCT_MUTATION = YES_BOUNDED_FOUNDATIONS_ONLY
PHASE_2_AUTHORIZED = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
NOTION_RECONCILIATION_REQUIRED = YES
```

## Freezes (binding)

- `PROXY_DP_ISOLATION = SEPARATE_PROCESS`
- `COMPETITIVE_H1_ACCEPT_OWNER = DATAPLANE_PROCESS`
- `DUAL_ACCEPT_COMPETITIVE_H1 = FORBIDDEN`
- `HYPER_FALLBACK_ON_COMPETITIVE_H1 = FORBIDDEN`
- `SDP_P1_RESURRECTION = FORBIDDEN`
- `PXDP_EXECUTION_RESURRECTION = FORBIDDEN_WITHOUT_NEW_OWNER_DECISION`
- `CAP067_REWRITE = FORBIDDEN`
- Feature gate default **OFF**

## Integrity

- No smoke as terminal evidence
- No fake READY / stub generation success
- No path/bench special-case for unfinished dataplane product traffic
- Netcup AMD64 required for material Phase-1 Linux qualification
- Oracle ARM64 not required unless platform-specific primitives diverge

## Feature gate

```text
EXYONQ_COMPETITIVE_H1_DATAPLANE=1   # enable foundations launch from exyonq serve
EXYONQ_CFD_LISTEN=IP:PORT          # exclusive competitive H1 listen (≠ product listens)
EXYONQ_CFD_GEN_DIR=/path           # generation + status directory (0700)
EXYONQ_CFD_SHARDS=N                # optional override
EXYONQ_DATAPLANE_BIN=/path         # optional; else sibling of exyonq / PATH
```

When OFF: current product path unchanged.  
When ON: fail-closed if binary missing, listen collides, or READY timeout — never transparent Hyper competitive H1 fallback.

## Notion delta (for owner/ChatGPT)

Record Phase 1 admission under Competitive Frontier:

1. Parent CFR-B closed → Phase 1 authorized.
2. Packaging: `exyonq` + `exyonq-dataplane`.
3. Gen publish: atomic file `EXYQCFD1` schema v1.
4. Gate default OFF; Phase 2 still unauthorized even if CFD-A.
5. Link tracked docs under `docs/architecture|governance|performance/v044-competitive-frontier-dataplane-foundations*`.

## Evidence root

`.exyonq-local/evidence/competitive-frontier-dataplane-foundations/<run-id>/`
