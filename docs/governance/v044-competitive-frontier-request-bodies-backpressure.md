# Governance — Phase 4 request bodies + bounded backpressure

```text
WIP = V044_COMPETITIVE_FRONTIER_REQUEST_BODIES_BOUNDED_BACKPRESSURE
PARENT_CASE = CFW3-B
PRODUCT_MUTATION = YES_BOUNDED_PHASE_4_ONLY
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
PHASE_5_AUTHORIZED = NO
PHASE_5_AUTOSTART = NO
PUSH = NO
TAG = NO
RELEASE = NO
NOTION_RECONCILIATION_REQUIRED = YES
```

## Ledger (cumulative)

| Phase | Case | Notes |
|-------|------|-------|
| Foundations | CFD-A | separate process |
| H1 routing | CFH2-B | |
| PCR-B / CM-B | PCR-B / CM-B | historical RPS/CPU/RSS GET |
| Phase 3 WAF | CFW3-B | WAF real; A0/A1 cost PARTIAL |
| **Phase 4 bodies** | **CFB4-B** | CL streaming + fail-closed chunked; GET frontier held; body-heavy Netcup YELLOW |
| **Phase 4 hygiene + P2** | **P4HC-B** | fmt/clippy/tests clean; L4-P2-001 absolute header deadline fixed; P3 ledger non-blocking |

Do **not** rewrite historical Phase 0–3 numeric claims.

Hygiene close detail: `docs/governance/v044-phase4-hygiene-residual-p2-close.md`.


## Integrity

```text
NO_FAKE_BODY_RELAY = required
NO_SMOKE = required
ANTI_KOBAYASHI = required
BENCHMARK_ONLY_BODY_PATH = FORBIDDEN
```
