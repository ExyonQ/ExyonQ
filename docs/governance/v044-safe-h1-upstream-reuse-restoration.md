# V044 — Safe H1 upstream reuse restoration

```text
WIP = V044_SAFE_H1_UPSTREAM_REUSE_RESTORATION
CASE = URR-A
ADR021 = docs/governance/v044-adr021-safe-h1-upstream-reuse-restoration.md
PRODUCT_MUTATION = YES (CFD shard-local pool only)
LOGIC_AUDITOR = PASS
NETCUP_AMD64 = PASS
```

## Historical truth (preserved)

| Epoch | Upstream put | Evidence |
|-------|--------------|----------|
| Phase-2 | Live (hit-rate overstated) | UPRR / CFD-L06 HISTORICAL |
| Phase-4 | Fail-closed discard-only | SOURCE_PROOF (trailing-byte race) |
| Pre-this-WIP | `put` dead; success always DISCARD | UPRR-C / UPRR-R6 |
| This WIP | Safe Hybrid F restore | ADR-021 + tests |

UPRR-C remains standing as the reconciliation of the discard-only era. This document records the **restoration**, not a rewrite of that history.

## Design (Hybrid F)

```text
CHOSEN_DESIGN = F_HYBRID
NO_NEW_PATTERN = YES
CHECKIN = dual nonblocking probe + idle READABLE register
IDLE = any readiness ⇒ DISCARD
CHECKOUT = gen match + probe + hold READABLE until first request write
POISON = trailing / EOF / timeout / parse / gen mismatch / pre-write readable
CHUNKED = still FAIL_CLOSED unsupported
EOF_FRAMED = never reuse
BODYLESS = reusable when !Connection: close
```

## Local evidence (Darwin iteration)

```text
cargo test -p exyonq-cfd-dataplane --test phase4_bodies
  clean_upstream_keepalive_reuses_single_accept              PASS
  delayed_trailing_byte_forces_fresh_accept                  PASS
  checkout_window_trailing_never_parsed_as_response          PASS
  upstream_trailing_bytes_do_not_corrupt_next_keepalive...   PASS
cargo clippy -p exyonq-cfd-dataplane --all-targets -- -D warnings  PASS
```

## Netcup amd64 evidence (AUTHORITATIVE for Linux)

```text
HOST = netcup-bench (x86_64)
TREE = /root/exyonq-urr-h1-reuse (synced CFD dataplane sources)
cargo test -p exyonq-cfd-dataplane --test phase4_bodies --release
  clean_upstream_keepalive_reuses_single_accept     PASS
  delayed_trailing_byte_forces_fresh_accept         PASS
  checkout_window_trailing_never_parsed_as_response PASS
  upstream_trailing_bytes_do_not_corrupt_...        PASS
LOGIC_AUDITOR_VERDICT = PASS
CHECKOUT_WRITE_TOCTOU = CLOSED
DIRTY_CONNECTION_REUSE_PATH = CLOSED
```

Evidence dir: `.exyonq-local/evidence/safe-h1-upstream-reuse-restoration/`

## Competitive invariants preserved

```text
TOKIO_THREADS_IN_DATAPLANE = 0
HYPER_ON_COMPETITIVE_H1_PATH = 0
CONTROL_PLANE_RPC_PER_REQUEST = 0
NEW_GLOBAL_HOT_LOCK = 0
SHARD_LOCAL_POOL = YES
```

## Performance

Narrow ExyonQ-only recheck is authorized only after Netcup correctness seal.
Historical P5CPU-A / EXT-A remain valid **pre-repair** baselines (UNCHANGED).

## Phase 6

```text
PHASE_6_AUTHORIZED = NO
BROAD_P1_P14_BENCHMARK = NOT_AUTHORIZED
```
