# L4 stream proxy (`[[stream]]`, NG-20)

ExyonQ can forward raw TCP or UDP traffic to an existing `[[upstream]]`
endpoint set. This is the L4 path used by MySQL, Redis, DNS or a QUIC endpoint:
ExyonQ does not parse the application protocol and does not terminate TLS.

## Configuration

```toml
config_version = 1

[[server]]
listen = "0.0.0.0:8080"
routes = []

[[upstream]]
name = "redis-pool"
timeout_ms = 30000

[[upstream.endpoints]]
id = "redis-a"
address = "192.0.2.10"
port = 6379
weight = 1

[[upstream.endpoints]]
id = "redis-b"
address = "192.0.2.11"
port = 6379
weight = 1

[[stream]]
name = "redis"
listen = "0.0.0.0:6379"
protocol = "tcp"          # tcp (default) | udp
upstream = "redis-pool"
idle_timeout_ms = 60000   # 1..=86400000
max_sessions = 4096       # UDP only; 1..=65536
```

| Key | Required | Default | Bound / notes |
| --- | --- | --- | --- |
| `name` | yes | — | Unique across `[[stream]]`; used in logs and validation. |
| `listen` | yes | — | Literal `SocketAddr` (`IP:port` or `[IPv6]:port`). |
| `protocol` | no | `tcp` | `tcp` or `udp`. |
| `upstream` | yes | — | Existing `[[upstream]]` name. |
| `idle_timeout_ms` | no | `60000` | `1..=86400000`; closes an idle TCP relay or UDP session. |
| `max_sessions` | no | `4096` | `1..=65536`; UDP session cap per listener. TCP ignores it. |

The IR accepts at most 128 `[[stream]]` blocks. A `[[stream]]` fails
validation if its upstream is missing, empty, or has no enabled endpoint with
weight greater than zero.

## Binding rules

L4 binds are fail-closed: every listener is bound before the HTTP accept loop
starts, so a bad address prevents startup instead of leaving a silently absent
surface. TCP and UDP are separate port spaces, so an HTTP listener on TCP/443
and a QUIC UDP listener on UDP/443 can coexist. Two TCP listeners cannot share a
TCP address, two UDP listeners cannot share a UDP address, a TCP stream cannot
duplicate the HTTP TCP bind, and a UDP stream cannot duplicate the `http3_listen`
UDP bind.

## Dataplane behaviour

- Selection reuses the proxy endpoint tables: smooth weighted round-robin,
  priority bands, endpoint weights and `admin_state`. `least_conn`, cookie and
  header affinity are HTTP policies and are not applied to L4.
- A TCP connection is pinned to one endpoint for its lifetime. If the connect
  fails, the next selected endpoint is tried.
- A UDP source `SocketAddr` is pinned to one endpoint while its session is
  alive. A session has a bounded 64-datagram queue; excess datagrams are
  dropped instead of growing memory without bound.
- `upstream.timeout_ms` bounds endpoint resolution/connect. TCP relays use
  `tokio::io::copy_bidirectional` (bounded buffers); UDP uses a 65,535-byte
  datagram buffer.
- Each accepted TCP connection and UDP session is admitted through the
  lifecycle counter and is closed on process shutdown.

## Reload

The L4 listeners and their endpoint selectors are built at startup. Changing a
`[[stream]]` block, or any upstream it references, requires a process restart:
`reload` returns `EXY-RELOAD-0005` rather than continuing to serve the old
selector under a new generation.

## Verification

- `config/ir/tests/stream_config.rs` — parse/defaults, bounds, unknown
  protocol/upstream, duplicate binds/names, TCP+UDP same-port acceptance, and
  fingerprint participation.
- `core/tests/ng20_stream_proxy_test.rs` — real local TCP/UDP backends: TCP
  weighted balancing, idle close and UDP client affinity.
- `core/tests/ng20_quic_relay_test.rs` — real `quinn` QUIC server and client:
  a full TLS/QUIC handshake (ALPN `ng20-quic`) and a bidirectional application
  stream through the UDP relay, with one UDP source pinned to the same backend
  across two connections and a second source selecting the other backend.
- `scripts/e2e/ng20-mysql-e2e.sh` — two real MySQL 8.4 servers and a real
  `mysql` client container through a TCP stream: `SELECT 1`, balanced
  `@@server_id` across both servers, one session reporting `VERSION()`, and
  connect failover after stopping one server.
- `scripts/e2e/ng20-stream-e2e.sh` — real `exyonq` binary and real sockets:
  TCP round-robin, UDP affinity, Redis `PING`/`SET`/`GET` through L4, config
  validation and listener startup.

**Native Linux (2026-11-16, evidencia de este tree): PASS amd64/arm64.** The
current tree was synced to Netcup (Ubuntu 24.04.4, x86_64, kernel
6.8.0-147-generic) and Oracle (Ubuntu 24.04.4, aarch64, kernel
7.0.0-1013-oracle), built there with `cargo build`, and the harnesses ran
without Docker Desktop or QEMU: `stream_config` 9/9, `ng20_stream_proxy_test`
3/3, `ng20_quic_relay_test` 1/1, `RESULT=PASS NG20_STREAM_E2E` (TCP
`Aping,Bping,Aping,Bping`, UDP `Audp/Audp/Budp`, Redis `PONG` + `SET/GET=ok`)
and `RESULT=PASS NG20_MYSQL_E2E` (two real MySQL 8.4 servers, balanced
`@@server_id` `4202,4201`, session `1 / 4202 / 8.4.13`, failover). Measured
binaries: 429 002 664 B / SHA-256 `42b6f388dad3d480…` on amd64 and 416 235 280 B
/ SHA-256 `89d2fdfa753e86ce…` on arm64. `scripts/e2e/lib/host-provenance.sh`
reports `HOST_DOCKERENV=absent`, `HOST_DETECT_VIRT=kvm`,
`HOST_PROVENANCE=vm` on both hosts: native Linux inside a hypervisor VM, not a
container. **CI executed on 2026-11-16.** The branch that carries this
increment, `release/v0.9.2`, was pushed as `bdcdc3c..6ee38a5` and the `check`
lane ran on Linux throughout: [push run `43955904651`](https://github.com/ExyonQ/ExyonQ/actions/runs/43955904651)
is **success** on `6ee38a5` (fmt, `clippy -D warnings`, `nextest` of the whole
workspace on `ubuntu-latest`, `static-linux-arm64` on `ubuntu-24.04-arm`,
`wasm-host`, `supply-chain`, `semver-checks` and
`build x86_64-unknown-linux-gnu`), and the same commit is green in
[PR #80 (`43955913574`)](https://github.com/ExyonQ/ExyonQ/actions/runs/43955913574),
[Security `43955913314`](https://github.com/ExyonQ/ExyonQ/actions/runs/43955913314)
and [HTTP benchmark `43955913133`](https://github.com/ExyonQ/ExyonQ/actions/runs/43955913133).
No macOS job was added to the lane: the tier-1 macOS build stays `main`-only, so
a release branch goes green with Linux runners alone. `E2E artifact-bound`
([`43955904588`](https://github.com/ExyonQ/ExyonQ/actions/runs/43955904588))
and `CRS differential`
([`43955913155`](https://github.com/ExyonQ/ExyonQ/actions/runs/43955913155))
were still red on that commit from causes that predate this increment: both
were already red on `bdcdc3c` on 2026-11-10, `OL-04` passes natively on the
bench host (13/13, `RESULT=PASS OL-04`) and fails only in the runner. They stay
open and are not attributed to the stream proxy. The per-run logs live in
`target/ng20-*.log` (untracked); the tracked record is
`docs/releases/0.9.2-release-evidence.md`.
