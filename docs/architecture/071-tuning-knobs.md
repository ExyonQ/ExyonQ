# Tuning knobs (NG-21)

ExyonQ runs without tuning: an absent `[tuning]` table keeps the values the
server already uses. The section exists so an operator can pin runtime geometry
without environment variables.

## `[tuning]`

```toml
config_version = 2

[tuning]
accept_workers = 4
```

| Key | Type | Bound | Default |
| --- | --- | --- | --- |
| `accept_workers` | integer | `1..=16` | detected CPU parallelism, clamped to `1..=16` |

### `accept_workers`

Accept / Cap067 prepare worker count. Resolution order:

1. `[tuning].accept_workers` when present;
2. `EXYONQ_ACCEPT_WORKERS` (`auto` or a positive integer);
3. `available_parallelism()` clamped to `1..=16` (affinity/cpuset aware on Linux).

The value is validated at parse time, so `0` or `17` fail `exyonq validate`
instead of being silently clamped at runtime. It is part of the IR fingerprint,
so changing it changes the reload generation. The Linux epoll keepalive pool
follows the resolved accept count unless `EXYONQ_EPOLL_POOL_THREADS` overrides
it.

Absent `[tuning]`, the server behaves exactly as before this section existed.
v1 configs keep parsing; the section does not require `config_version = 2`.

## What is deliberately not exposed yet

NG-21 also names keepalive, timeouts and buffers. Those knobs currently live in
process-lifetime caches read from `EXYONQ_READ_TIMEOUT_MS` and in a compile-time
header cap, shared by the Hyper path, the static wire path and the Linux epoll
keepalive path. Moving them into the IR requires a per-generation runtime
plumbing refactor across those consumers; publishing the fields before that
wiring would create config keys that change nothing, which this project does not
ship. They stay pending in NG-21.

## Defaults and measurement

The default is the pre-existing geometry (`V044_P1_DEFAULT_WORKER_GEOMETRY_8`):
one worker per detected CPU up to 16. The memory harness for defaults is
`scripts/e2e/worker-memory-e2e.sh`, which compares idle and concurrent RSS
against `exyonq_worker_pool_resident_memory_bytes{workers="N"}` on native Linux.

Measurement status: **NOT EXECUTED in this increment.** It needs a native Linux
amd64/arm64 host; Docker Desktop on macOS is not canonical Linux evidence, so no
RSS figure is claimed here.

## Verification

- `config/ir/tests/tuning_config.rs` — parse, absent-means-auto, bounds rejection,
  unknown keys, fingerprint participation, v1 compatibility.
- `core/tests/tuning_accept_workers.rs` — the config value reaches the runtime
  resolver and wins; defensive clamp at the edges.
- Real binary: `exyonq serve` with `[tuning] accept_workers = 2` must log
  `workers=2` in the `exyonq listening` line and still serve traffic;
  `exyonq validate` must reject `accept_workers = 17`.
