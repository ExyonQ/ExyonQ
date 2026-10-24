# V044 Phase 6D — FastCGI resource/concurrency stress qualification

```text
WIP                          = V044_PHASE6D_FASTCGI_RESOURCE_CONCURRENCY_STRESS_QUALIFICATION
PHASE                        = 6D
PARENT_TERMINAL              = P6C-B
CASE                         = P6D-C
STATUS                       = COMPLETE_WITH_PRODUCT_REPAIR_REQUIRED
DATE_UTC                     = 2026-08-30
PLATFORM                     = LINUX_AMD64_NETCUP
PRODUCT_MUTATION             = NO
PRODUCT_REPAIR_REQUIRED      = YES
PERFORMANCE_OPTIMIZATION     = NO
WORDPRESS                    = NO
PHASE_6E_AUTHORIZED          = NO
PHASE_7_AUTHORIZED           = NO
PUSH                         = NO
TAG                          = NO
RELEASE                      = NO
PUBLICATION                  = NO
```

## Purpose

Qualify the post-P6C CFD-native FastCGI path under real PHP-FPM concurrency and resource pressure. Not a feature, performance, WordPress, or rival-benchmark phase.

## Entry freeze

```text
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
BINARY_SHA256 (VALID run) = 926c628932a0dc33adb595bfad2d7265cb86f404c8ad2dc89b827d5035f6fbcf
  # Matches P6C terminal binary; rebuilt on Netcup from ambient CFD surface
ORACLE_ARM64_REQUIRED = NO
  # Tranche claim = Netcup amd64 qualification only; dual-arch before shipping / VERIFIED_REAL_PRODUCTION
```

Ambient dirty tree declared under evidence root (`AMBIENT_DIRT_DECLARATION.txt`). P6D owns harness/docs/evidence only; CFD product surface is parent-readonly for this WIP.

## Runs disclosed

| RUN_ID | CLASS | Note |
|--------|-------|------|
| `20260830T193154Z` | INVALID | Incomplete FPM tree kill (orphaned workers kept listen); SUMMARY aborted on self-`cp`; retained |
| `20260830T194007Z` | VALID | Supersedes INVALID for stress matrix; full listen-tree kill → 502 |

Evidence root: `.exyonq-local/evidence/phase6d-fastcgi-resource-concurrency-stress-qualification/`

## What held (BOUNDED Netcup amd64)

- Real PHP-FPM + dynamic identity oracle; contamination = 0 on gated stages
- S0 SYN=1/50; S8 long-lived KEEP_CONN 500 requests / 1 SYN with client `Connection: close`
- Concurrency through 32 without contamination
- S6 hard backend kill (full listen tree) → HTTP 502; recovery without ExyonQ restart
- S7 generation publish under load → gen 3 observed
- FD/RSS/thread series: FD 6→7, RSS plateau ~4104 KiB, threads flat at 5; TOKIO=0 HYPER=0
- Residual `FCGI_POST_PROBE_TO_FIRST_WRITE_MICRO_WINDOW` preserved (not reopened)
- Dirty reuse on PHP-FPM path remains `NOT_MEASURED` (honest)

## Product defect — MUST_FIX (STOP)

```text
FINDING_ID = P6D-L1-001
SEVERITY   = LOGIC-P1
TITLE      = FcgiPoolMap::ensure_pool sticks idle_ttl/max_connections across reload
FILE       = crates/exyonq-cfd-dataplane/src/fcgi_pool.rs
ROOT       = HashMap::or_insert_with ignores later CompiledFcgiPool idle_ms/max_conn for same pool_id
EVIDENCE   = S8 published idle_ms=2000; sleep 3s; S8_evict SYN=0 (still using first-seen 60s idle)
CONTRACT   = published generation pool limits must govern runtime after reload
PRODUCT_MUTATION_IN_THIS_WIP = FORBIDDEN (owner repair authorization required)
```

## Harness / claim hygiene (non-product)

Independent logic/GRC audits also require SUMMARY honesty: several counters were stamped `0`/`PASS` without oracles (`BACKEND_GRACEFUL_RESTART`, generation-invalid reuse, duplicate-exec, retry-after-commit). Treat those as `NOT_MEASURED` until instrumented. S8 long-reuse PASS remains supported; S8 idle-eviction leg is **not** proven while P6D-L1-001 is open.

## Audits

| Agent | Verdict |
|-------|---------|
| Orchestrator | CONDITIONAL_ACCEPT (qualification-only) |
| Vertex architect | CONDITIONAL_ACCEPT |
| Security (terminal) | PASS_WITH_NONBLOCKING |
| GRC (terminal) | FAIL (SUMMARY missing→PASS/0) |
| Independent logic | FAIL_PROVEN_DEFECTS (P6D-L1-001) |

## Claim ceiling

```text
MAX_CLAIM = BOUNDED_LINUX_AMD64_RESOURCE_CONCURRENCY_STRESS_QUALIFICATION
VERIFIED_REAL_PRODUCTION = NOT_CLAIMED
SHIPPING = NOT_CLAIMED
ALL_PLATFORMS = NOT_CLAIMED
FASTCGI_PERFORMANCE_FRONTIER = NOT_MEASURED
```

## Next phase

```text
NEXT_PHASE_AUTHORIZED = NO
RECOMMENDED_NEXT =
  Owner-authorized product repair of P6D-L1-001 (ensure_pool refresh) + harness SUMMARY honesty
  then re-run Netcup S8 eviction + S9 pool-bound stages before any Phase-6E / WordPress tranche
WORDPRESS_FUTURE_QUALIFICATION = CONDITIONAL
  # Blocked on pool reload config correctness + residual ledger
```

## Docs

- Terminal block: `docs/governance/v044-phase6d-fastcgi-resource-concurrency-stress-qualification-terminal-block.txt`
- Harness: `scripts/reality/run-phase6d-fastcgi-resource-concurrency-stress-netcup.sh`
- Parent: ADR-039 / P6C-B terminal
