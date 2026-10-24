# Governance — Competitive Frontier Phase 3 WAF + generation reload

```text
WIP = V044_COMPETITIVE_FRONTIER_WAF_GENERATION_RELOAD
CASE = CFW3-B
PRODUCT_MUTATION = YES_BOUNDED_PHASE_3_ONLY
HARNESS_MUTATION = YES
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
PHASE_4_AUTHORIZED = NO
PHASE_4_AUTOSTART = NO
PUSH = NO
TAG = NO
RELEASE = NO
NOTION_RECONCILIATION_REQUIRED = YES
```

## Ledger (cumulative — do not rewrite historical PCR-B / CM-B numbers)

| Phase | Case | Disposition |
|-------|------|-------------|
| CFD foundations | CFD-A | closed |
| CFH2 H1 routing | CFH2-B | YELLOW |
| PCR-B multi-conn | PCR-B | YELLOW OWNER_REVIEW (RPS/P95/P99) |
| CM-B CPU/RSS accounting | CM-B | OWNER_REVIEW (CPU/RSS axes) |
| **Phase 3 WAF+gen** | **CFW3-B** | **OWNER_REVIEW** — WAF real; frontier vs rivals held; incremental cost PARTIAL |

## Integrity declarations

```text
DOES_CHANGE_PRODUCT_SEMANTICS = YES (CFD Phase3 only)
DOES_ADD_BENCHMARK_SPECIAL_CASE = NO
DOES_ADD_PATH_SPECIAL_CASE = NO
DOES_ADD_LOADGEN_DETECTION = NO
DOES_ADD_IMPLICIT_CACHE = NO
NO_FAKE_WAF = PASS
NO_SMOKE = PASS
ANTI_KOBAYASHI = PASS_WITH_DISCLOSURES
```

## Harness mutation

```text
WHY_REQUIRED = A0/A1 WAF on/off + rival frontier on Netcup; control harness publishes representative WAF
SEMANTIC_EFFECT = none on nginx/OLS product configs; HAProxy sidecar http-reuse always (parity with CM-B)
DOES_IT_FAVOR_EXYONQ = NO
```

## Notion delta

1. Phase 3 CFD ships compiled WAF projection + composite generation reload.
2. Netcup: Phase3 WAF-ON median RPS ≈ 160.7k vs OLS ≈ 109.4k (**+46.9%**); CPU ≈ 12.1 vs 23.1 µs/req (**+47.7%** advantage).
3. A0→A1 incremental cost **not** claimed isolated (A1 RPS median higher than A0; order bias / noise).
4. P95/P99 **NOT_MEASURED** this campaign.
5. Phase 4 **not** started.

## Related

- Architecture: [`../architecture/v044-competitive-frontier-waf-generation-reload.md`](../architecture/v044-competitive-frontier-waf-generation-reload.md)
- Performance: [`../performance/v044-competitive-frontier-waf-generation-reload-budget.md`](../performance/v044-competitive-frontier-waf-generation-reload-budget.md)
- Parent accounting: [`v044-phase2-cpu-memory-accounting-close.md`](v044-phase2-cpu-memory-accounting-close.md)
