# OL-03 — Per-client throttling

OpenLiteSpeed exposes this surface in WebAdmin as **Per Client Throttling**;
the native configuration block is `perClientConnLimit` (not
`perClientThrottling`). ExyonQ maps every field to
`[modules.ratelimit.client]`:

```toml
[modules.ratelimit.client]
enabled = true
soft_connections_per_ip = 10000
max_connections_per_ip = 10000
bandwidth_bytes_per_second = 131072
input_bandwidth_bytes_per_second = 131072
static_requests_per_second = 20
dynamic_requests_per_second = 10
block_bad_req = true
grace_seconds = 15
ban_seconds = 300
```

## OpenLiteSpeed mapping

| OpenLiteSpeed `perClientConnLimit` | ExyonQ `modules.ratelimit.client` | Behaviour |
| --- | --- | --- |
| `softLimit` | `soft_connections_per_ip` | concurrent TCP/QUIC connections may exceed it for `grace_seconds`, then the IP is banned |
| `hardLimit` | `max_connections_per_ip` | immediate close without an HTTP response at the ceiling; a hard ban also aborts live bodies |
| `gracePeriod` | `grace_seconds` | time over the soft limit before a ban is armed |
| `banPeriod` | `ban_seconds` | ban duration for soft-limit and malformed-request offenders |
| `outBandwidth` | `bandwidth_bytes_per_second` | response-body bytes/second per IP |
| `inBandwidth` | `input_bandwidth_bytes_per_second` | request-body bytes/second per IP |
| `staticReqPerSec` | `static_requests_per_second` | static-route requests/second; excess is deferred |
| `dynReqPerSec` | `dynamic_requests_per_second` | proxy/FastCGI requests/second; excess is deferred |
| `blockBadReq` | `block_bad_req` | ban an IP after repeated malformed HTTP requests |

`soft_connections_per_ip` requires `max_connections_per_ip`; any pair is
accepted, including `softLimit > hardLimit` and equality. OpenLiteSpeed's parser
accepts both and its hard check wins; ExyonQ preserves the same precedence
without rejecting the block. `block_bad_req = true` requires `ban_seconds >= 1`.
Inbound and outbound bandwidth values must be at least 1024 bytes/s when set.
Enabling the block requires at least one actual limit.

## Translation path

There is a real offline importer for this block:

```bash
exyonq-compat ols-throttle \
  --input httpd_config.conf \
  --out throttle.toml \
  --report translate.md
```

It accepts a complete OLS configuration and extracts exactly one
`perClientConnLimit` block. Zero or multiple blocks, unknown fields, duplicate
fields, malformed integers, nested blocks, integer fields outside
OpenLiteSpeed's own ranges (`softLimit`/`hardLimit`/`banPeriod` below 1,
`gracePeriod` outside 1..=3600), rate fields above `INT_MAX`, and `blockBadReq`
values other than 0/1 are rejected. A block with only `hardLimit` is valid; the
absent `softLimit` becomes `INT_MAX`. Nothing is silently ignored.

Translation follows the OLS defaults and units:

- absent `softLimit`/`hardLimit` → `2147483647` (`INT_MAX`, OpenLiteSpeed's
  parser default, i.e. no limit); absent `gracePeriod` → `10`; absent
  `banPeriod` → `60`. These are the defaults of the OpenLiteSpeed 1.9.4 parser
  (`getLongValue(node, name, 1, max, default)` in
  `src/main/httpserver.cpp`), not the WebAdmin form values;
- explicit `softLimit`, `hardLimit`, `gracePeriod` or `banPeriod` below `1`, and
  `gracePeriod` above `3600`, are rejected: OpenLiteSpeed's own range check
  would not accept them either;
- a hard-only block is valid because OpenLiteSpeed's parser requires only that
  each present value is in range; absent limits default to `INT_MAX`;
- `outBandwidth` is rounded up to 4 KiB, `inBandwidth` to 1 KiB;
- an absent `inBandwidth` follows `outBandwidth`, as OLS does; an explicit
  `inBandwidth 0` disables inbound throttling;
- a zero request rate or zero bandwidth omits that limit;
- the output is a mergeable `[modules.ratelimit.client]` fragment, not a claim
  to migrate unrelated OLS directives.

## Runtime semantics

- **Request quotas:** fixed one-second windows per IP, separately for static
  and dynamic routes. The first `N` requests in a window pass; the excess waits
  for the next available slot instead of receiving `429`. A static route is one
  whose backend is `Backend::Static`; proxy/FastCGI routes are dynamic.
- **Concurrency:** one slot per accepted TCP connection (H1/H2) or QUIC
  connection (H3), acquired before serving requests and held until the socket
  closes. HTTP/2 streams share their parent connection; H3 uses the peer address
  from the provider. The hard limit closes the accepted socket without writing
  an HTTP response and arms the same ban as OLS; bodies admitted before that
  refusal abort on their next poll, matching OLS truncation of live responses.
  The soft limit admits while inside `grace_seconds`, then bans and closes the
  connection that observes the expired grace. The grace marker is cleared under
  the same live-count/half-soft/within-grace condition as OLS's timer.
- **Bandwidth:** per-IP token buckets, one for response bodies and one for
  request bodies. The bucket starts with one second of capacity and large data
  frames are split into throttled chunks, so the first frame is emitted
  immediately instead of waiting for the whole body. Upload throttling applies
  to H1/H2 streaming bodies; the already-buffered H3 entry point keeps its
  aggregate delay before dispatch. At 32 KiB/s and a 256 KiB response, OLS took
  7.015 s and ExyonQ 7.018 s; the first body byte arrived at 0.011 s and 0.005 s
  respectively (2026-11-05, same host).
- **`blockBadReq`:** a protocol-parse failure is counted per IP. Three
  malformed requests accumulate until the same `ban_seconds` ban is armed (the
  counter then resets); the next connection is closed without an HTTP response,
  and the IP recovers when the ban expires. Incomplete client disconnects are
  not counted. OLS 1.9.4 exposes the option in WebAdmin but has no server-side
  implementation, so this remains an ExyonQ extension with a deterministic
  threshold.
- **Scope and bounds:** process-local, per client IP, shared across routes and
  listeners. State is capped at 10,000 IPs; inserting a new IP evicts one
  existing entry. Bans survive reloads but not a restart.

## Deliberate differentials

These are not hidden as parity:

- Both implementations count TCP connections. ExyonQ applies the same guard to
  QUIC/H3 connections, which have no OLS counterpart in the tested oracle.
- Both close the new connection at the hard limit without a response. A
  hard-limit ban also aborts bodies admitted before the refusal. In the live A+B+C
  run, OLS sent 32509 body bytes on A, 0 on B and 0 on C; ExyonQ sent 32768 on A,
  11047 on B and 0 on C. The close/truncation semantics match; the exact point at
  which the live bodies stop differs by a fraction of a refill interval.
- Concurrent A/B downloads have the same aggregate rate and both complete, but
  OLS serves A first and delays B's first data until A finishes, while ExyonQ
  interleaves token chunks. This is response scheduling, not a different limit:
  totals were 15.280 s (OLS) and 15.379 s (ExyonQ), 262144 body bytes each.
  It is declared here rather than claimed as byte-for-byte parity.
- `blockBadReq` exists only in OpenLiteSpeed's WebAdmin
  (`dist/admin/html.open/lib/LSWebAdmin/Product/WebServer/DTblDefBase.php`) and
  its help pages; the 1.9.4 server sources contain no implementation of it.
  ExyonQ uses a deterministic threshold of three. OpenLiteSpeed's
  `allowedRobotHits` (`lsrecaptcha`, default `3`) and `s_iMaxAllowedBotHits = 3`
  are the closest numeric source evidence, but they govern bot marking, not
  `blockBadReq`; the threshold is not claimed as `blockBadReq` parity.
- The importer used to reject `softLimit > hardLimit`; that was a translator
  defect, not an OLS rule. It now accepts and preserves the pair, and the hard
  check wins in both implementations.
- The importer is intentionally limited to `perClientConnLimit`; it does not
  claim a complete OpenLiteSpeed migration.

## Verification (2026-11-03, historical)

Final debug binary `target/debug/exyonq`, Darwin arm64, SHA-256
`c9036ba0e986c583098e9181413acc7d0169788474a81a4d4ba0e5a4e8c0f07f`;
compat binary SHA-256
`d5afa9eb4cc08481233b57fb51f2a11c2c0614f571d84cd5b2bc0a2b773afac7`.

- OL-03 full E2E: **22/22 PASS**, including OLS block extraction, validation,
  static RPS deferral, dynamic RPS deferral, 1 MiB inbound upload at 128 KiB/s
  (5.162 s), 1 MiB outbound response at 128 KiB/s (5.702 s), three real
  malformed requests → 429 ban and recovery, immediate hard-limit 429, and the
  soft/grace/ban journey (second request inside grace 200, then 429, then 200
  after ban).
- AP-04 E2E: **28/28 PASS** with the same binary.
- CRS differential gate: **failures=0/4628**, `excluded_with_evidence=33/4628`.
- `cargo fmt --all -- --check`: PASS.
- Clippy `-D warnings` on the touched crates: PASS.
- `exyonq-compat-openlitespeed`: 7/7; `exyonq-config-ir`: 182/182;
  `exyonq-core --lib client_throttle`: 7/7.

Evidence: `.exyonq-local/evidence/ol03/full-final3.log`,
`.exyonq-local/evidence/ol03/full-20261103T213123Z/`,
`.exyonq-local/evidence/ap04/ap04-final2.log`,
`.exyonq-local/evidence/ap04/crs-final2.log`.

## Verification (2026-11-04, historical)

This section records the field-level work before the connection-semantics
change; its 22/22 run and its `429` checks are historical and are superseded by
the 2026-11-05 section below.

Criterion, literal from `Roadmap 0.9.1` (OL-03, line 48):

> *Criterio de aceptación:* `perClientThrottling` migrado 1:1

Executed on the tree at `fb33e81` plus this increment's working-tree changes.
Nothing below is inferred from the 2026-11-03 runs.

| Criterion surface | Result | Harness assertion(s) |
| --- | --- | --- |
| one `perClientConnLimit` block extracted from a full OLS config | PASS | `OLS translation exit` |
| `staticReqPerSec` window, excess deferred | PASS | `static RPS statuses`, `static RPS defers excess` |
| `dynReqPerSec` window, excess deferred | PASS | `dynamic RPS statuses`, `dynamic RPS defers excess` |
| `outBandwidth` (4 KiB units) throttles a 1 MiB response | PASS | `outbound size`, `outBandwidth throttles 1 MiB response` |
| `inBandwidth` (1 KiB units) throttles a 1 MiB upload | PASS | `inBandwidth throttles 1 MiB upload` |
| `softLimit` + `gracePeriod` → ban, then recovery | PASS | `soft limit allows request inside grace`, `soft limit arms ban after grace`, `soft ban remains active`, `soft ban expires` |
| `hardLimit` rejects a concurrent request | PASS | `hard limit rejects concurrent request`, `hard-limit slow request completes` |
| `banPeriod` expires the ban | PASS | `blockBadReq ban expires`, `soft ban expires` |
| `blockBadReq` bans after malformed requests | PASS | `blockBadReq bans after malformed requests` |
| translated fragment passes `exyonq validate` | PASS | `validate translated OLS configuration` |

These rows verify field-level scope on the historical binary; they are not a
1:1 verdict. The final live OLS comparison and the closing verdict are in the
2026-11-05 section below.

### Darwin arm64

```bash
bash scripts/e2e/ol03-full-e2e.sh
```

- **22/22 PASS**, `RESULT=PASS OL-03 FULL`, exit 0.
- `target/debug/exyonq`
  `51b87e2f11eb18812325e34087931d16b72122f18a09bf2b1a909e671b0d7229`;
  `target/debug/exyonq-compat`
  `f50580e4c6537e70b2f91d8c0b022980b5b27177029c4075a9cc0358577fee50`.
- Samples: 1 MiB upload 5.392 s; 1 MiB download 5.546 s.
- Log `.exyonq-local/evidence/ol03/full-20261104T163339Z.log`, SHA-256
  `8e983dd6e0411e77f9ccde3f1d3a193c6282ea16724ffca537e0b5547c1b84e7`.

### Linux arm64

Built from the same tree in a Wolfi container (rustc 1.99.1 `58cf8f9c3`,
`aarch64-unknown-linux-gnu`) with
`cargo build --locked -p exyonq --bin exyonq -p exyonq-compat-cli --bin exyonq-compat`;
both artifacts are `ELF 64-bit LSB pie executable, ARM aarch64, … for GNU/Linux
4.9.0`.

- `exyonq`
  `0cc88c562eadb99e5be2a88fd67155446b0308a170843e4084a9a34221e7f158`;
  `exyonq-compat`
  `a31eba7b3f41a76261e0016125582abb742dbd780bba38226cb1ff72b635cc98`.
- Build log `.exyonq-local/evidence/ol03/linux-build-20261104T163328Z.log`,
  SHA-256 `4401e48e8dff9432b08a40ed6e8674fa0977af485a6d43ce8222ed38786d8c3f`,
  `DOCKER_EXIT=0`.
- Same harness, `exyonq-test:wolfi`, `Linux 7.0.14-linuxkit aarch64`:
  **22/22 PASS**, `RESULT=PASS OL-03 FULL`. Samples: 1 MiB upload 4.178 s;
  1 MiB download 5.372 s (1048576 bytes).
- Log `.exyonq-local/evidence/ol03/linux-full-20261104T163657Z.log`, SHA-256
  `b66d348c1c21bad5fbebff4f6bed1c5358af307bda2112b1bb6374021880f105`; run
  directory `.exyonq-local/evidence/ol03/linux-run-20261104T163657Z/`.

Environment note: the first Linux attempt (`linux-full-20261104T162810Z.log`,
SHA-256 `661939338b7de0731547a30faaad4e2a4c6755c9b8c6f1a5a829ebed70fc266d`)
never reached the first check. The control socket lives in the run directory and
`chmod` on the shared virtiofs mount returns `EINVAL`
(`Error: control socket bind failed: chmod owner-only on control socket …:
Invalid argument (os error 22)`). Moving the run directory to the container's
local filesystem passed 22/22. The product refused to start instead of serving
without its control plane; this is a limitation of the shared mount, not a
change in product behaviour.

### Checks

- `cargo test -p exyonq-compat-openlitespeed --locked`: **10/10**, including
  three new regressions for the OpenLiteSpeed parser defaults and range checks.
- `cargo test -p exyonq-config-ir --locked`: **182/182**;
  `cargo test -p exyonq-core --lib client_throttle --locked`: **7/7**.
- `cargo clippy -p exyonq-compat-openlitespeed -p exyonq-config-ir -p exyonq-core
  --all-targets --locked -- -D warnings`: PASS.
- `cargo fmt --all -- --check`: PASS.
- `scripts/verify-no-private-paths.sh`: `OK: no private paths in git index`.
- `scripts/security/scan-private-material.sh --publication --repo .`:
  `PRIVATE_MATERIAL_FINDINGS=0`, `PRIVATE_MATERIAL_GATE=PASS` (modo canónico:
  incluye untracked y blobs staged).
- Anti-theater gates under `scripts/gates/`, exit code per command:
  `no-smoke-as-proof-gate.sh --tree .` → exit 0;
  `zero-smoke-category-gate.sh --tree .` → exit 0;
  `real-evidence-gate.sh --files …` → exit 0.

### Still open

- Native Linux amd64/arm64 lab hosts (Netcup/Oracle) were not used: no access
  from this workstation. The Linux arm64 run above is a container on a real
  aarch64 Linux kernel, not a native lab host; Linux amd64 is **NOT EXECUTED**,
  because an emulated run would make the timing assertions meaningless.
- A side-by-side differential against a running OpenLiteSpeed binary was **NOT
  EXECUTED at 2026-11-04**. It was executed against OLS 1.9.4 on 2026-11-05;
  the result is below and supersedes this limitation for the tested surfaces.

## Verification (2026-11-05, final tree)

Criterion literal: *`perClientThrottling` migrado 1:1* (`Roadmap 0.9.1`, OL-03).
HEAD `fb33e81c272456a434a0641273143c8fff419c6f`, branch `release/v0.9.1`, with
the working-tree changes from this increment. Host Darwin arm64.

- Darwin arm64 `target/debug/exyonq` SHA-256
  `db7a07afd843e6e87554fb1d09d2e1375a21916e00950b971ea1c4849a87ad6c`;
  `target/debug/exyonq-compat` SHA-256
  `e98ace9572fb2dbab284a42c752c5088fe656538224d82a418b8d668fc882a34`.
- Final `scripts/e2e/ol03-full-e2e.sh` on Darwin: **25/25 PASS**, exit 0. Log
  `.exyonq-local/evidence/ol03/e2e-final-20261105T220627Z.log`, SHA-256
  `93c7344fdf7b205707dccb85325012aa70ee2f535ba94cfbe9c0ac754df769d9`.
- Linux arm64 Wolfi (Rust 1.99.1, `aarch64-unknown-linux-gnu`) built from the
  same working tree: `exyonq`
  `398b544ed9431d0c99d771d6416a13231415a6f9212cc8988bc6353260897b2e`,
  `exyonq-compat`
  `db167b03b67a86c1cb422c1cb35df34d3bb02282f6f8ba1145896eecf64f3726`; same
  harness: **25/25 PASS**, exit 0, log
  `.exyonq-local/evidence/ol03/linux-e2e-20261105T221532Z.log`, SHA-256
  `103cb54f6cbc908626f825e4e718343a79e965b537594252e4fcccae289770eb`.
- `cargo check -p exyonq-core --lib` in a Linux 1.99.1 container: PASS. This
  compiles the Linux-only `connection_executor` path that the Darwin build does
  not see.
- `cargo test -p exyonq-core --lib`: **242/242**; `client_throttle` **11/11**;
  `ps1a_h3_lifecycle` **3/3**; `exyonq-mod-http3` **4/4**; workspace
  `cargo check --all-targets`: PASS; Clippy `-D warnings` on core, module-api,
  mod-http3 and both providers: PASS; `cargo fmt --check`: PASS.
- Real OLS oracle: `litespeedtech/openlitespeed:latest`, index digest
  `sha256:91ca324386be3fe8cd91591112dbdd8e41af48cccbe4a48e3e8673a2921c8504`,
  OLS **1.9.4**, Linux arm64, source commit `031b2c8`; container
  `eq-ols-oracle` on `127.0.0.1:19080`, `httpdWorkers 1`.

### Live OLS comparisons (2026-11-05, final binary)

The oracle was restarted before each case so an earlier soft-limit marker could
not contaminate the next run. Configuration in OLS 1.9.4 and ExyonQ was identical:
`softLimit/soft_connections_per_ip = 1`, `hardLimit/max_connections_per_ip = 2`,
`gracePeriod/grace_seconds = 2`, `banPeriod/ban_seconds = 3`,
`outBandwidth/bandwidth_bytes_per_second = 32768`,
`inBandwidth/input_bandwidth_bytes_per_second = 32768`, and both request rates 0.
The served file was 262144 bytes.

| Case | OLS 1.9.4 | ExyonQ | Observable result |
| --- | --- | --- | --- |
| one connection | 7.015 s, 262144 body bytes, first data 0.011 s | 7.018 s, 262144 body bytes, first data 0.005 s | same total, same unit, first frame immediate in both |
| A at 0 s + B at 0.35 s | 15.280 s; A completed at 6.937 s, B at 15.280 s; both 262144 | 15.379 s; A at 11.024 s, B at 15.379 s; both 262144 | same aggregate rate and completion; different concurrent scheduling |
| A+B+C (`Connection: close`) | C closed with 0 bytes; A truncated at 0.905 s with 32509 body bytes; B 0 body bytes; total 1.316 s | C closed with 0 bytes; A truncated at 1.019 s with 32768 body bytes; B 11047 body bytes; total 1.356 s | hard limit closes the third connection and aborts live bodies in both |
| A at 0 s + B at 2.2 s (soft) | both completed; A 262144 at 6.946 s, B 262144 at 15.216 s | both completed; A 262144 at 9.023 s, B 262144 at 15.234 s | B is admitted during grace; no divergence in the limit decision |

OLS container `eq-ols-oracle`, `litespeedtech/openlitespeed:latest`
(`sha256:91ca324386be3fe8cd91591112dbdd8e41af48cccbe4a48e3e8673a2921c8504`),
OLS 1.9.4, source commit `031b2c8`, `httpdWorkers 1`. The OLS-side source
observation for `checkAccess`/`resetQuotas` was downloaded under
`.exyonq-local/evidence/ol03/ols-source-20261105/`; the probe inputs and raw JSON
are in `.exyonq-local/evidence/ol03/ols-compare-20261105/`.

### Verdict

The nine fields are implemented and exercised against the final Darwin and Linux
binaries. The unit and effects now match the oracle: **connection**-level
concurrency for H1/H2 and a per-QUIC-connection lease for H3, close without a
response at the hard limit, hard-ban truncation of live bodies, soft grace then
ban, ban recovery, immediate first frame, and per-IP bandwidth totals. The only
remaining declared differential is response scheduling between concurrent A/B
downloads: OLS serves A first, ExyonQ interleaves chunks, with the same aggregate
time and bytes. `blockBadReq` remains an ExyonQ extension because OLS 1.9.4 has no
server-side implementation to compare. `OL-03` is therefore **closed for 0.9.1**
on the documented `perClientConnLimit` contract, without claiming byte-for-byte
scheduling parity outside that contract.

### Reproduction (2026-11-05T222329Z)

`bash scripts/e2e/ol03-ols-compare.sh` recreated the oracle comparison with a
raw-TCP probe and restarted `eq-ols-oracle` before every scenario. It verified
the oracle config (`soft=1 hard=2 grace=2 ban=3 out=32768 in=32768`) and passed
all four cases:

| Case | OLS | ExyonQ |
| --- | --- | --- |
| single | 6.998 s, 262144 body bytes, first data 0.005 s | 7.017 s, 262144 bytes, first data 0.004 s |
| A+B | 14.966 s, both 262144 bytes | 15.380 s, both 262144 bytes |
| A+B+C | C closed with no frame; A 32509 B 32509 bytes, both < 262144 | C closed with no frame; A 32768 B 11511 bytes, both < 262144 |
| soft (B at 2.2 s) | both 262144 bytes, 15.206 s | both 262144 bytes, 15.230 s |

The current AP-04 gate is **30/30**, OL-04 is **11/11** and the OL-03 E2E is
**25/25** on the same binary `db7a07af…`. Workspace nextest is **2185 passed, 2
skipped**; `cargo check --workspace --all-targets --locked`, Clippy with
`-D warnings`, and `cargo fmt --check` all pass. The complete logs and SHA-256
manifests are in `.exyonq-local/evidence/repro-20261105T222329Z/` and
`.exyonq-local/evidence/ol03/ols-repro-20261105T222544Z/`.

The first nextest build hit the host file-descriptor limit
(`Too many open files in system`); the reported run used `ulimit -n 8192`,
`CARGO_BUILD_JOBS=4` and `--test-threads=4`. This was an environment limit, not
a test failure. `real-evidence-gate.sh --tree .` still reports historical local
evidence debt; `--diff HEAD` and `--files` over this increment pass.

### Revalidación 2026-11-06

Repetición sobre el mismo árbol y binario `db7a07af…`. AP-04 pasó **30/30** y
OL-04 **11/11**; OL-03 produjo **24/25** en el primer intento porque el burst
estático medido nada más crear la conexión de `/health` heredó su ventana de un
segundo y el tercer request terminó en `0.717 s`, por debajo del umbral `0.75 s`.
La medición aislada confirmó que era timing del harness, no un bypass: `0.661 s`
con la ventana heredada y `1.032 s` tras una ventana limpia. OLS reinicia las
cuotas desde su timer y la creación del cliente, por lo que el test ahora espera
`1.05 s` tras el probe antes de medir y exige `>= 0.9 s`; no se cambió el
producto. El rerun dio **25/25 PASS**, `RESULT=PASS OL-03 FULL`, exit 0
(`ol03-rerun-console.log`, SHA-256 `9e29abad…`).

OL-05 se reejecutó con el gate único sobre CRS `v4.25.0`, corpus
`1fb44865…` y el oráculo por digest: bloque `0/4628` con 33 exclusiones
evidenciadas, dry-run `0/4628`, `would_block=5947`, `violations_942100=52`,
métricas block/detection-only PASS y `FILES`/`XML` 11/11; `OL05_VERIFY=PASS`,
exit 0, binario `db7a07af…`. La cláusula «en CI» sigue abierta.

### Audit stack (2026-11-04)

Same tree, after the runs above. Veyra `standard` → **ACCEPTED**, exit 0
(`.veyra/evidence/20261104T163957Z/report.md`); geiger INCONCLUSIVE and dylint /
semver-checks / veyra-e2e SKIPPED for the reasons recorded in
`docs/waf/challenge.md`, section *Audit stack*. Rust Audit exited 1 against
`report/rust-audit-20261104-180101.md`; the only finding in this increment's
throttle file is R08-medium, the 1.1 s sleep in
`soft_limit_ban_is_armed_after_grace` (`core/src/client_throttle.rs:453`), which
waits past a 1 s soft grace. It is a unit test with a 100 ms margin; the product
path does not sleep. CodeGraph leg 3 is closed in two parts: the full structural graph (637 files,
24 155 nodes) yields a cycle verdict of **no crate/module cycles**
(`Imports`/`Uses`/`Extends`/`Implements` empty; only intentional intra-file
mutual recursion) with exit 0; and the scoped vector store (1 154 embeddings)
lets `agent context --focus search` return `file:line` for the throttle
(`core/src/client_throttle.rs:95/134/192/277/295`) and for the WAF challenge
(`crates/exyonq-waf/src/challenge.rs:146/208/289/333`). Evidence and hashes are
in `docs/waf/challenge.md`, section *Audit stack*. The vector store is scoped to
the WAF/throttle paths for convergence; the full graph is kept as
`.codegraph/db.full-<timestamp>`. Hindsight **NOT EXECUTED** (MCP not loaded).
