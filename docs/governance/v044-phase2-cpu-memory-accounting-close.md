# Governance — Phase-2 CPU/memory accounting close

```text
WIP = V044_PHASE2_CPU_MEMORY_ACCOUNTING_CLOSE
CASE = CM-B
PRODUCT_MUTATION = NO
HARNESS_MUTATION = YES
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
PHASE_3_AUTHORIZED = NO
PHASE_3_AUTOSTART = NO
NOTION_RECONCILIATION_REQUIRED = YES
```

## Ledger update (cumulative frontier)

| Phase | Disposition | Notes |
|-------|-------------|-------|
| CFD-A foundations | closed | separate-process geometry |
| CFH2-B Phase2 H1 | YELLOW | single-conn not comparable |
| PCR-B multi-conn | YELLOW OWNER_REVIEW | RPS/P95/P99 strong; CPU rivals NOT_MEASURED; RSS dataplane-only |
| **CM-B accounting close** | **OWNER_REVIEW** | Rival CPU + Phase2 combined RSS measured; frontier STRONG; Phase3 still owner-gated |
| Phase 3 WAF+gen (CFW3-B) | OWNER_REVIEW | See `v044-competitive-frontier-waf-generation-reload.md` — does **not** rewrite CM-B numbers |

## Notion delta

1. Close PCR-B CPU/memory evidence gaps with dual-method Netcup campaign.  
2. Phase2 combined CPU ≈ 11.7 µs/req vs OLS ≈ 23.3 (+49.9%).  
3. Phase2 shipping RSS ≈ 5.6 MiB (control+dataplane) vs nginx ≈ 31.6 MiB / HAProxy ≈ 20.3 MiB.  
4. Total frontier STRONG vs SPPG targets; **do not auto-start Phase 3**.  
5. Historical PCR-B RPS/P95/P99 values remain authoritative for those axes.

## Harness mutation declaration

```text
WHY_REQUIRED = obtain rival cgroup+/proc CPU; Phase2 control+dataplane RSS/CPU; HAProxy keepalive equivalence
SEMANTIC_EFFECT = none on ExyonQ/nginx/OLS product configs; HAProxy sidecar adds http-reuse always only
DOES_IT_CHANGE_WORKLOAD = NO
DOES_IT_FAVOR_EXYONQ = NO
DOES_IT_FAVOR_A_COMPARATOR = NO
```
