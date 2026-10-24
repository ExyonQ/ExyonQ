# V044 — Specialized proxy event-loop control (architecture)

```text
WIP = V044_SPECIALIZED_PROXY_EVENT_LOOP_CONTROL
PARENT = GA-B
CASE = EP-C
MODE = ISOLATED_NON_PRODUCT_ARCHITECTURE_PROTOTYPE
PRODUCT_MUTATION = NO
CONTROL_IS_PRODUCT = NO
```

Governance: [`../governance/v044-specialized-proxy-event-loop-control.md`](../governance/v044-specialized-proxy-event-loop-control.md)  
Performance: [`../performance/v044-specialized-proxy-event-loop-control.md`](../performance/v044-specialized-proxy-event-loop-control.md)  
Evidence: `.exyonq-local/evidence/specialized-proxy-event-loop/20260827-183032/`  
Source: `.exyonq-local/experiments/specialized-proxy-event-loop/` (ADR-021)

---

## Primary question

```text
CAN_A_SPECIALIZED_RUST_EVENT_LOOP_PROXY_EXECUTION_MODEL_MATERIALLY_OUTPERFORM
  EXYONQ_PXDP_AND_THE_MINIMAL_HYPER_TOKIO_CONTROL_ON_P4
  WITHOUT_REQUIRING_A_WHOLE_PRODUCT_REWRITE =
  YES
```

---

## ADR-021 selection

| Option | Verdict |
|--------|---------|
| A mio single-loop | Insufficient alone for 8-CPU matrix |
| **C TPC + mio** | **SELECTED** |
| B Tokio current-thread shards | Weaker falsifier of Hyper/Tokio class |
| D TPC + Hyper low-level | Still Hyper I/O model |
| E Hyper/Tokio multi-worker | Already MC-B |

```text
SELECTED_MODEL = C_THREAD_PER_CORE_PLUS_MIO
ALIAS = MIO_STYLE_SHARDED_EVENT_LOOP_H1_PROXY
```

Properties: SO_REUSEPORT per shard; connection owned for lifetime; shard-local upstream idle sockets; httparse H1; no per-request tasks/channels/work-stealing; safe Rust; no io_uring.

LOC = **925** (≤1000 target). Deps: `mio`, `httparse`, `socket2`.

---

## Execution geometry

| Model | OS threads | Tasks/req | Channels/req | Cross-shard handoffs | Client owner | Upstream owner |
|-------|------------|-----------|--------------|----------------------|--------------|----------------|
| ExyonQ PXDP (w4) | ~6 (Tokio+notify) | ≥1 async | present in product path | work-stealing possible | Tokio task / Hyper | Hyper client |
| Hyper/Tokio control (w4) | ~5 | ≥1 spawn/conn | Hyper internal | work-stealing | Tokio task | Hyper pool |
| **Prototype (shards=4)** | **5** (4 shards + main wait) | **0** | **0** | **0** | **shard loop** | **same shard** |
| NGINX (conceptual) | workers | 0 | 0 | 0 (conn local) | worker event loop | same worker |

---

## Integration feasibility (read-only, EP-C)

```text
INTEGRATION_FEASIBILITY = GOOD
```

Desired future shape (not implemented):

```text
ExyonQ product/control (Tokio) — routing / WAF / generations / observability
        | immutable snapshots / decisions
        v
specialized mio TPC proxy dataplane (H1 hot path only)
```

| Input | Feasibility |
|-------|-------------|
| Routing decision | Snapshot / precompiled plan into shard |
| WAF decision | Admit-time verdict before upstream write |
| Generation | Arc/RCU publish; no hot mutex |
| Observability | Async off-path counters/events |
| Errors | Map dataplane codes → product error class |
| Hyper | Remain for control API / H2 / non-hot roles |
| Tokio | Remain process/control substrate (GA-B) |
| Cap067 | Untouched |

```text
WHOLE_PRODUCT_REWRITE_REQUIRED = NO
PRODUCT_LOGIC_REWRITE_REQUIRED = NO
FOLLOW_ON_DESIGN = V044_SPECIALIZED_PROXY_DATAPLANE_CONTRACT_AND_INTEGRATION_DESIGN
FOLLOW_ON_CASE = SD-B (COMPLETE)
NEXT_PRODUCT_WIP_PROPOSAL = V044_SPECIALIZED_PROXY_DATAPLANE_SDP_P1_GET_KNOWN_LENGTH
PRODUCT_IMPLEMENTATION_AUTHORIZED = NO
DOC = docs/architecture/v044-specialized-proxy-dataplane-contract-integration-design.md
```

---

## Portability

| Platform | Strategy |
|----------|----------|
| Linux | epoll via mio (this evidence) |
| macOS | kqueue via mio (local iteration only) |
| Windows | IOCP via mio later — not in this WIP |

---

## Explicit non-claims

- Prototype is **not** product code and must not be merged as ExyonQ.
- Does not authorize removing Tokio/Hyper/Cap067.
- Does not implement routing/WAF/reload/TLS/H2/H3.
- Ctx absolute values for NGINX/OLS in this campaign are **INVALID** (perf attach undercount); RPS/CPU remain authoritative.
