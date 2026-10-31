# P11 TLS+H2 lab — blockers (2026-09-23)

```text
STATUS = BLOCKED_LAB
SCENARIO = P11
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
HOST = netcup-bench nodelayab stack
```

## Why P11 is not runnable on current nodelayab

| Blocker | Detail |
| --- | --- |
| No TLS listeners on rivals/ExyonQ in nodelay compose | Stack is cleartext `:8080` / OLS `:8088` |
| Prior matrix skip | `SKIP_NO_TLS_ON_NODELAYAB` in P1–P14 rivals matrix |
| No shared TLS fixture | Need server certs + SNI + h2 ALPN for ExyonQ + Tier-A rivals |
| Cap067 EPOLL path | TLS accept today Hyper/h2 branch in `server/mod.rs`; Cap067 wire-cheap AE-gate is cleartext-first |

## Minimal lab kickoff (when unblocked)

1. Overlay compose: ExyonQ + nginx + OLS (or HAProxy) with TLS on internal ports; mount bench PEMs.
2. wrk/h2load scenario for P11 path (`/api/` over TLS+H2 as roadmap defines).
3. DEV remasure only; official compare later under rule 104.
4. Do **not** claim Cap067 wire-cheap on TLS until early-gate + keepalive policy re-audited for TLS streams.

## Not in this tranche

- Product TLS bugfixes from INFORME TLS-001…004
- Official `bench compare --all-servers` publication
