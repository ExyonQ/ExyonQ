# ADR-021 implementation review — SDP-P1

```text
WIP = V044_SPECIALIZED_PROXY_DATAPLANE_SDP_P1_GET_KNOWN_LENGTH
PHASE = SDP-P1
PARENT = SD-B / EP-C / GA-B
STACK_LABEL = ADR-021_C + SD-B_B
PROTOTYPE_COPY = FORBIDDEN
```

## Comparison (before product mutation)

| ID | Option | Verdict for SDP-P1 |
| --- | --- | --- |
| **A** | mio + httparse hardened production shell (new crate) | **SELECTED** — matches SD-B B; EP-C geometry (ADR-021 C) without pasting 925 LOC |
| B | Sharded current-thread Tokio | REJECT for value gate — EP-C falsifier weaker than mio TPC |
| C | Hyper low-level H1 inside shard | REJECT — reintroduces Hyper geometry tax |
| D | Retain current PXDP / Hyper wire | REJECT as sole path — PR-C FAIL; remains fallback when SDP off |
| E | No new pattern | REJECT — no way to answer integration value question |

## Why A

- Preserves EP-C connection ownership (one shard for client/upstream life).
- httparse already in lockfile; mature header parse vs NIH.
- Portability via mio (epoll/kqueue).
- Bounded complexity: leaf crate `exyonq-proxy-sdp`, not core mio.
- Security: eligibility before irreversible specialized consume; CL/TE gates in parse.

## Expected complexity (pre-code)

```text
NEW_CRATE_LOC_ESTIMATE = 1200–2000
CORE_WIRING_LOC_ESTIMATE = 150–300
CLI_REGISTER_LOC = <50
TESTS_LOC_ESTIMATE = 400–800
NET_COMPLEXITY > 0 — justified by EP-C + P4 deficit; ceiling = no Cap067/module API rewrite
```

## Dependencies (owner §48)

| Crate | Spec | License | Role |
| --- | --- | --- | --- |
| mio | 1.x (lock ~1.2) | MIT | readiness |
| httparse | 1.x (lock 1.10.1) | MIT/Apache-2.0 | H1 headers |
| socket2 | 0.6 (match platform-linux) | MIT/Apache-2.0 | REUSEPORT |
| arc-swap | workspace | MIT/Apache-2.0 | generation publish |

No mio in `exyonq-core`.

## Feature gates

```text
EXYONQ_SDP_P1=1          # default OFF — SDP owns cleartext accept
EXYONQ_SDP_SHARDS=N      # optional; else derived from parallelism (no magic global 4)
```
