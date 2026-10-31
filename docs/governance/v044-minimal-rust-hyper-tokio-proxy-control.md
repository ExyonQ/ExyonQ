# V044 — Minimal Rust Hyper+Tokio proxy control (governance)

```text
WIP = V044_MINIMAL_RUST_HYPER_TOKIO_PROXY_CONTROL
MODE = ISOLATED_NON_PRODUCT_CONTROL_EXPERIMENT
PRODUCT_MUTATION = NO
EXYONQ_PRODUCT_CODE_MUTATION = NO
CONTROL_IS_PRODUCT = NO
CONTROL_CANONICAL_BUILD_MEMBER = NO
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN
PUBLICATION = NO
```

## Authority

Owner-authorized differential control experiment. **Not** product code, **not** merge candidate, **not** Hyper/Tokio replacement authorization.

Primary question: under canonical P4 proxy 1 KiB workload, does a minimal Hyper+Tokio H1 reverse proxy exhibit the same high context-switch and CPU geometry as ExyonQ PXDP, or is the gap primarily ExyonQ-specific integration?

## Scope allowed

| Allowed | Forbidden |
| --- | --- |
| Standalone control under `.exyonq-local/experiments/minimal-hyper-tokio-proxy/` | ExyonQ routing, modules, reload, observability |
| Same P4 harness, upstream, cpuset `0-7`, Netcup amd64 | Product semantic shortcuts |
| Read-only ExyonQ binary (P5 terminal image) | Request-body relay, PXDP P6 |
| Evidence under `.exyonq-local/evidence/minimal-hyper-tokio-control/` | Commit/push/tag/release |

## Decision cases (MC-A … MC-E)

See performance doc for measured outcome and interpretation gates.

## Integrity

- Real TCP, real Hyper, real upstream, real rewrk — `ZERO_FAKE=YES`
- Perf scope: all process threads (never single-PID ctx under-count)
- Invalid prior evidence `a1-20260827-022600` (~7.9 ctx/req) = `INVALID_SCOPE_MISMATCH` — do not cite

## Related

- Performance report: [`../performance/v044-minimal-rust-hyper-tokio-proxy-control.md`](../performance/v044-minimal-rust-hyper-tokio-proxy-control.md)
- Prior attribution: [`../performance/v044-pxdp-scheduler-wakeup-topology-attribution.md`](../performance/v044-pxdp-scheduler-wakeup-topology-attribution.md) (**SW-C**)
