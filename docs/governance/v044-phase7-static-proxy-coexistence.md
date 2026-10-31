# V044 Phase 7 — Static + Proxy + FastCGI coexistence

```text
WIP = V044_PHASE7_STATIC_PROXY_COEXISTENCE
CASE = P7-E
PARENT_TERMINAL = P6G-A (IMMUTABLE)
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
VALID_RUN = 20260831T225032Z
PLATFORM = LINUX_AMD64_NETCUP
BINARY_SHA256 = 5d4b6bdf6c6439f0b98656e1e84c83ea6a5cd0640490bf310e9e8af7197b367b
PRODUCT_MUTATION = NO
HARNESS_MUTATION = YES_EVIDENCE_ONLY
TEST_MUTATION = NO
DOC_MUTATION = YES
PHASE_8_AUTHORIZED = NO
```

## Question

Can **CURRENT** ExyonQ CFD serve, in one published generation:

1. static filesystem resources  
2. reverse-proxy routes  
3. FastCGI / application routes  

with deterministic ownership, precedence, isolation, failure boundaries, and reload?

## Answer (bounded)

```text
THREE_HANDLER_NATIVE_COEXISTENCE = NO
REASON = BackendKind::Static ABSENT from CFD IR
PROXY_PLUS_FASTCGI_SAME_GENERATION = PASS (Netcup)
STATIC_NATIVE_FILESYSTEM_SERVE = FALSIFIED_ABSENT (Netcup)
```

P7-A is unreachable without a product static handler. This WIP does **not** authorize repair.

## Current route precedence (extracted, not invented)

From `crates/exyonq-cfd-gen/src/route_table.rs` (Cap033):

```text
CURRENT_ROUTE_PRECEDENCE =
  exact host > wildcard host > hostless
  then longest path prefix
BACKEND_KINDS = Proxy | Fastcgi | Reject
```

Publish CLI documents **proxy + FastCGI** only (`cfd-publish-routes`).

Non-PHP files that exist under a FastCGI-matched path resolve to
`FcgiRouteResolve::NotFound` → HTTP 404 (`fcgi_route.rs`) — they are **not**
served as filesystem static content.

## Cap067

Cap067 is **product-plane** epoll static residency (`EXYONQ_EPOLL_STATIC`), not a CFD
dataplane handler. With `PRODUCT_MUTATION=NO`, Cap067 product semantics are
preserved by non-touch. CFD has no Cap067-equivalent native static path to qualify.

```text
CAP067_STATIC_PATH = NOT_APPLICABLE_CFD_NO_NATIVE_STATIC
PRODUCT_PLANE_CAP067 = PRESERVED_NO_MUTATION
```

## P6G static labeling (honesty)

P6G `TARGET_APP_STATIC_ASSET` / `STATIC_PRECEDENCE` used **Proxy** routes to a
Python `SimpleHTTPRequestHandler` on `/assets/`. That proves longest-prefix
**proxy** ownership over FastCGI catch-all — **not** a CFD native static handler.

Phase 7 forbids reusing that pattern as `STATIC` proof (`ZERO_FAKE` /
`ANTI_KOBAYASHI`).

## Valid run evidence

```text
EVIDENCE = .exyonq-local/evidence/phase7-static-proxy-coexistence/20260831T225032Z
HARNESS = scripts/reality/run-phase7-static-proxy-coexistence-netcup.sh
```

| Probe | Result |
| --- | --- |
| Publish same gen: `/api/` Proxy + `/app/` FastCGI + `/` FastCGI | PASS |
| GET existing `/static/hello.txt` | **404**; body SHA ≠ file SHA |
| GET missing `/static/missing…` under `/` FC | **200 FASTCGI** (front-controller catch-all) |
| Proxy GET/POST/query/404/500 | PASS |
| Proxy 302 without upstream `Content-Length` | 502 framing (**VALID_RUN** harness) |
| Proxy 302 with `Content-Length: 0` | PASS (**OUT_OF_PACKAGE** diagnostic; not sealed FIX) |
| FastCGI front-controller / query / POST | PASS |
| Proxy↔FastCGI marker contamination | MEASURED_0 (**sequential** only) |
| Three-way STATIC+PROXY+FCGI | **FAIL** (STATIC absent) |

## Workspace gates (attributable)

CFD packages on Netcup foundations tree:

```text
CARGO_FMT_CHECK = PASS_CFD_PACKAGES
CARGO_CHECK = PASS (cfd-dataplane/gen/control/fastcgi-wire)
CARGO_CLIPPY_CANONICAL = PASS_CFD_PACKAGES
cargo test -p exyonq-fastcgi-wire = PASS
cargo test -p exyonq-cfd-dataplane --test phase6b_fcgi =
  PRE_EXISTING_FAIL trailing_data_forces_discard_error (not introduced by P7)
```

Full `--workspace` ambient gates not treated as Phase 7 attribution (dirty tree).

## Specialist consensus (post-evidence)

| Agent | Verdict |
| --- | --- |
| Orchestrator | `CONDITIONAL_ACCEPT` — L2 evidence-only; defect → P7-C STOP; no PASS without Netcup |
| Vertex | `CONDITIONAL` — Proxy+Fastcgi Cap033 OK; native three-handler Static **INSUFFICIENT** |
| Research | `PARTIAL` — migration value PARTIAL if Static claimed as native |
| Security | `CONDITIONAL` — Proxy-as-STATIC/Cap067 mislabel = HIGH handler confusion |
| GRC | `CONDITIONAL` — **RECOMMENDED_CASE=P7-E**; P7-A via proxy-as-static = ZERO_FAKE FAIL |
| Logic auditor | `PASS_WITH_NONBLOCKING_P3_FINDINGS` — `P7_FALSIFIED=NO`; keep P7-E |

## Terminal taxonomy

| Case | Meaning | Selected |
| --- | --- | --- |
| P7-A | Three-way coexistence VERIFIED_REAL_PRODUCTION | **NO** |
| P7-B | Coexistence with nonblocking qualifications | **NO** — three-way not materially proven |
| P7-C | Supported product defect | **NO** — Static was never a supported CFD `BackendKind`; absence is a **capability gap**, not a broken supported contract |
| P7-D | Environment blocked | **NO** |
| **P7-E** | Insufficient for coexistence VERIFIED | **YES** |

## Migration value

```text
DOES_PHASE7_INCREASE_MIGRATION_VALUE = PARTIAL
```

Operators migrating NGINX/Apache/OLS **static + API proxy + PHP** topologies still
lack a first-class CFD static filesystem handler. Proxy+FastCGI same-generation
isolation is real and useful, but does **not** equal three-handler coexistence.

## Recommended next WIP (NOT authorized)

```text
RECOMMENDED_NEXT_WIP = OTHER_REQUIRED_PREREQUISITE
OTHER = CFD_NATIVE_STATIC_FILESYSTEM_HANDLER_ARCHITECTURE_ADMISSION
NEXT_WIP_AUTHORIZED = NO
```

Architecture admission completed under separate WIP
`V044_CFD_NATIVE_STATIC_FILESYSTEM_HANDLER_ARCHITECTURE_ADMISSION`
(**CASE P7STATICADM-B**, ADR-044) — docs only; `BackendKind::Static` code still
**NOT_AUTHORIZED**. Recommended follow-on (still owner-gated):
`V044_CFD_NATIVE_STATIC_FILESYSTEM_HANDLER_IMPLEMENTATION`.

Do not start Phase 8, async FastCGI, or performance frontier from this terminal.

## Preserved parents

```text
P6G-A / P6F-A / P6EROUTE-A / CAP067 / TTL_RELOAD / GENERATION_RELOAD /
MAX_CONN_CONFIG_HONESTY / FCGI_MAX_CONN=1 / SYMLINK_TOCTOU BOUNDED_RESIDUAL
```

## STOP

Owner review required. No merge / push / tag / release / publication / Phase 8.
