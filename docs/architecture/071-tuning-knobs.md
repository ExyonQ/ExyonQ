# Tuning knobs (NG-21)

ExyonQ runs without tuning: an absent `[tuning]` table keeps the values the
server already uses. The section exists so an operator can pin runtime geometry
without environment variables.

## `[tuning]`

```toml
config_version = 2

[tuning]
accept_workers = 4
keepalive_timeout_ms = 30000
max_header_bytes = 8192
```

| Key | Type | Bound | Default |
| --- | --- | --- | --- |
| `accept_workers` | integer | `1..=16` | detected CPU parallelism, clamped to `1..=16` |
| `keepalive_timeout_ms` | integer (ms) | `1..=600000` | `EXYONQ_READ_TIMEOUT_MS`, else 30000 |
| `max_header_bytes` | integer (bytes, power of two) | `8192..=65536` | 8192 |

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

### `keepalive_timeout_ms`

The keep-alive idle deadline and the header-read deadline are the same wait: a
connection that stops sending the next header block is closed after it. One
value governs every HTTP/1.1 path — the core pre-parser
(`core/src/server/io.rs`), the static wire, the proxy wire and the Hyper
connection the wire hands over — so a 2 s keep-alive does not expire at 30 s on
one path and at 2 s on another. HTTP/2 and HTTP/3 keep their own deadlines: the
HTTP/2 builder does not take this value, and the HTTP/3 providers expose only
the QUIC idle timeout. That split stays open in NG-21.
Resolution order: `[tuning].keepalive_timeout_ms`, then `EXYONQ_READ_TIMEOUT_MS`,
then 30 s. The env var stays for deployments that already pin it; the config key
wins so a single file describes the server.

### `max_header_bytes`

The largest request-header block the server accepts. It bounds the pre-parser
buffers, the static and proxy wire readers, the Linux epoll `ConnState` buffer,
and Hyper's `max_buf_size`, so the cap is the same whichever component reads the
the next request. Hyper's default buffer is replaced by this value. It must be a
power of two in `8192..=65536`; a malformed value fails `exyonq validate`
instead of being clamped.

### Live behaviour

Both values are published by `exyonq_module_api::runtime_tuning` from
`reload::publish_runtime_observe_authority`, i.e. only when a generation commits.
A reload that changes either key changes the fingerprint, so it produces a new
generation; a rejected reload leaves the live geometry untouched. Both keys are
skipped when absent, so a config that does not set them keeps the exact
fingerprint it had before they existed, and no stored generation hash is
invalidated by the upgrade. Connections
already open keep their own deadline until they close, which is the same
contract as every other per-generation value.

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
- `core/tests/tuning_header_geometry.rs` — the committed geometry reaches the
  container header timeout and each commit republishes it.
- `scripts/e2e/ng21-tuning-e2e.sh` — real binary: a 12 KiB header block is
  refused with the default cap and served with `max_header_bytes = 16384`,
  keep-alive reuse inside `keepalive_timeout_ms = 400` and a close after 900 ms
  idle, plus `validate` rejecting `0`, `600001`, `4096` and `65535`.
- Real binary: `exyonq serve` with `[tuning] accept_workers = 2` must log
  `workers=2` in the `exyonq listening` line and still serve traffic;
  `exyonq validate` must reject `accept_workers = 17`.
