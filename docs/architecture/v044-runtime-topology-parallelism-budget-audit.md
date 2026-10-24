# V044 — Runtime topology & parallelism budget audit

```text
WIP = V044_RUNTIME_TOPOLOGY_AND_SECOND_TOKIO_POOL_ATTRIBUTION
MODE = READ_ONLY_RUNTIME_OWNERSHIP_AND_EXECUTION_TOPOLOGY_AUDIT
STATUS = CLOSED
CASE = RT-C
SOURCE_HEAD = 0bc2b973e5ff62b2316fbc7c3f002d673c65d791
SOURCE_TREE = b1b7659b4d3bb8745ea0ed42a8de7d424f523767
PARENT = WG-B / EC-A
PRODUCT_MUTATION = NO
```

Governance: [`../governance/v044-runtime-topology-parallelism-budget-audit.md`](../governance/v044-runtime-topology-parallelism-budget-audit.md)  
Performance companion: [`../performance/v044-runtime-topology-parallelism-budget-audit.md`](../performance/v044-runtime-topology-parallelism-budget-audit.md)  
Evidence: `.exyonq-local/evidence/runtime-topology-parallelism/20260827-173800/`

## Primary answer

**There is one production Tokio multi-thread runtime (RT-01) in `exyonq serve`.**

WG-B “Block B” (+8 `tokio-rt-worker` names) is **not** a second Tokio runtime. It is the **Cap067 keepalive OS-thread pool** (`exyonq_platform_linux::epoll_worker::start_keepalive_pool`), proven by `perf` DWARF stacks (`epoll_wait` → `start_keepalive_pool`). Those OS threads inherit the parent Tokio worker’s `comm` name because they are spawned with unnamed `std::thread::spawn` from a Tokio worker after lazy Cap067 start.

## Runtime ledger

| ID | Kind | Constructor | Workers | Control |
|----|------|-------------|---------|---------|
| **RT-01** | Tokio multi_thread | `cli/exyonq/src/main.rs` Builder | `EXYONQ_WORKER_THREADS` / `default_runtime_parallelism()` | env |
| **POOL-CAP067** | OS threads | `epoll_worker::start_keepalive_pool` | follows accept / `EXYONQ_EPOLL_POOL_THREADS` | env |
| notify-rs | OS thread | `notify` watcher | 1 | n/a |
| ConnPool | OS threads | `conn_pool` | default 64 | **inactive** on default P4 (`sync_accept=false`) |
| BlockingStaticPool | OS threads | mod-static | `avail*32` clamp 64–512 | static path only, lazy |

DB5 contract: sole production `Builder::new_multi_thread` is CLI. No production `Runtime::new` / nested multi_thread builders found.

## Block A / Block B

| Block | Owner | Formula | Proof |
|-------|-------|---------|-------|
| A | RT-01 | `EXYONQ_WORKER_THREADS` | TID cohort size; futex park stacks |
| B | Cap067 OS pool | `resolve_epoll_pool_threads(accept)` (=8 on cpuset 0–7 when unset) | perf → `start_keepalive_pool` |

## P4 request topology

Default cleartext P5 stack log: `epoll_keepalive=true`, `sync_accept=false`, `epoll_static(listen)=false`.

```text
client → RT-01 accept_loop task → Hyper/wire → ProxyConnection → Hyper upstream → response
```

- `PXDP_REQUEST_CROSSES_MULTIPLE_TOKIO_RUNTIMES = NO`
- Cap067 may be **process-active** (static/keepalive) but is **not proven** on the pure `/api/` proxy hot path.

## Accept / Cap067 / Conn pool

| Component | Model | P4 default |
|-----------|-------|------------|
| Accept | **Tokio tasks** on RT-01 (`tokio::spawn` × `resolve_accept_workers()`) | 8 tasks; **not** OS threads; **does not** create a Runtime |
| Cap067 | OS epoll workers | Active (prepared + lazy-started); 8 OS threads |
| Conn pool | OS threads (`EXYONQ_CONN_POOL_SIZE` default 64) | **Inactive** (`sync_accept=false`) |

## Parallelism fan-out

`default_runtime_parallelism()` independently sizes Tokio (when WT unset), accept, and Cap067. Setting **only** `EXYONQ_WORKER_THREADS` leaves accept+Cap067 at detected parallelism → WG-B `WORKER_COUNT_ISOLATED=PARTIAL` explained.

`GLOBAL_PARALLELISM_BUDGET_EXISTS = NO` — coordinated defaults (G1), not a single sum budget.

## Host projections (defaults, MAX=16)

| CPUs | Tokio | Accept tasks | Cap067 OS | ≈ OS threads (main+Tokio+Cap067+notify) |
|-----:|------:|-------------:|----------:|----------------------------------------:|
| 2 | 2 | 2 | 2 | 6 |
| 4 | 4 | 4 | 4 | 10 |
| 8 | 8 | 8 | 8 | 18 |
| 16 | 16 | 16 | 16 | 34 |
| 32/40/64 | **16** | **16** | **16** | **34** |

A 40-CPU host does **not** get 40 Tokio workers; each pool clamps at 16 independently.

## Design intent

Documented in G1 / `runtime_parallelism.rs` / governance `v044-p1-default-worker-geometry-8.md`: intentional coordination of **defaults**. Thread-name inheritance making Cap067 look like Tokio workers is **observability drift**, not a second Runtime.

## Classification

**CASE = RT-C** — independent parallelism fan-out proven.  
Not RT-B (no duplicate Tokio runtime). Block B owner **PROVEN**.

## Next experiment (proposal only)

`EXYONQ_EPOLL_STATIC=0` vs default on P4 — isolate Cap067 OS-thread process cost from Tokio worker scaling. Do **not** start multi-concurrency worker-policy sweep until that isolation is understood.

```text
MULTI_CONCURRENCY_SWEEP = HOLD
NEXT_PRODUCT_REPAIR = NONE
```
