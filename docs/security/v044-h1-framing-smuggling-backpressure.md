# Security — H1 framing / smuggling / backpressure (CFD Phase 4)

```text
CL_TE = REJECT_AND_CLOSE
DUPLICATE_CL = REJECT_AND_CLOSE
TRANSFER_ENCODING = FAIL_CLOSED (chunked not implemented)
BARE_CR_LONE_LF_IN_HEADERS = REJECT
FOUNDATION_WITH_BODY = FORCE_CLOSE
UPSTREAM_TRAILING_BYTES = POISON (no pool reuse after response — fail-closed)
```

Independent audit: `INDEPENDENT_SMUGGLING_AUDIT = PASS`  
Evidence: `.exyonq-local/evidence/competitive-frontier-request-bodies-backpressure/20260828-224420/security-audit.md`

## Slowloris / absolute header deadline (L4-P2-001)

```text
PRE_FIX = header `since` reset on every readable drip → unbounded acquisition
POST_FIX = absolute monotonic CLIENT_HEADER_TIMEOUT (5s); no drip refresh
REAL_TCP_PROOF = crates/exyonq-cfd-dataplane/tests/slowloris_header_deadline.rs
HEALTHY_TRAFFIC_UNDER_ATTACK = required PASS in same suite
```
