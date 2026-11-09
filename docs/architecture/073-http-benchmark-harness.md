# Public HTTP benchmark harness (Q-12)

Q-12 adds a public, reproducible HTTP/1.1 benchmark for ExyonQ against the
native servers selected for a run. Its purpose is a declared engineering
baseline with intervals and an anti-regression gate, not a competitive
scoreboard.

```text
measurement_class = local_native_non_canonical
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN unless separately authorized
```

## Components

| Path | Role |
| --- | --- |
| `scripts/bench/run-http-bench.sh` | Starts the selected servers, validates bodies, runs the load and writes raw evidence. |
| `scripts/bench/aggregate.py` | Validates every raw repetition and builds `report.json` / `report.md` with bootstrap intervals. |
| `scripts/bench/rss_sampler.py` | Samples the resident set of the whole server process tree every 100 ms. |
| `scripts/gates/bench-regression-gate.sh` | Compares two `report.json` files and fails on a throughput drop or p99 rise beyond the tolerance. |
| `.github/workflows/bench.yml` | Runs the native Linux benchmark in CI, publishes the `main` baseline and gates PRs against it. |
| `.github/workflows/bench-ols.yml` | Installs native OLS on Linux, serves the benchmark payload and runs the same harness with OLS. |

## Measurement protocol

- All selected servers run natively on the same host and kernel, bind loopback
  and read the same payload files. The payload directory is copied to a
  temporary directory because the macOS Apache build cannot read files under
  `/Volumes` (TCC/`EPERM`).
- The default payloads are `1k.bin` and `64k.bin`, both tracked in
  `benchmarks/scenarios/payloads/www`.
- HTTP/1.1 keep-alive is on, access logging is off and `sendfile` is enabled
  where the server supports it.
- Before any measurement, each server/payload pair is fetched with `curl` and
  its body is compared by exact SHA-256 against the source file.
- After a 2 s warm-up per cell, the harness runs 5 repetitions of 10 s per
  server/payload with a closed-loop `oha` load: 16 connections. `oha` 1.14 has
  no thread selector, so the report declares connections only.
- Server order rotates every repetition so slow drift is spread across the
  selected servers instead of favouring one.
- Every repetition must return only HTTP 200, `successRate = 1.0` and no
  transport or application errors; otherwise the run aborts.
- The resident set of the whole server process tree is sampled every 100 ms
  and the peak is reported.
- Results are the median over repetitions with a 95% bootstrap interval of the
  median (10,000 resamples, fixed seed). Raw repetitions remain on disk.

The 16-connection default is deliberate. On the reference macOS host the
system Apache build drops a small fraction of keep-alive connections at 32–64
concurrent connections, so 16 is the highest tested level where every cell
reached `successRate = 1.0` for the tested servers. Throughput is therefore
lower than a 64-connection run would suggest, but the comparison is valid on
the tested level and the limitation is explicit.

## Evidence layout

```text
OUT/
  raw/<payload>__<server>__r<N>.json        # oha JSON
  raw/<payload>__<server>__r<N>.rss.json    # RSS samples
  meta.json                                 # host, versions, binary SHA-256, git state
  report.json                               # machine-readable medians + intervals
  report.md                                 # human-readable report
  logs/                                     # server validation and runtime logs
```

`aggregate.py` is fail-closed: a missing repetition, a stale repetition, a
non-200 status, a success rate below 1.0, a zero/NaN metric or a missing RSS
file aborts aggregation. It never fills a missing cell with zero or with a
previous value.

## Anti-regression gate

```bash
bash scripts/gates/bench-regression-gate.sh \
  --baseline baseline/report.json \
  --candidate current/report.json \
  --max-rps-drop-pct 10 --max-p99-rise-pct 15
```

The gate compares the median RPS and p99 of every baseline cell with the same
cell in the candidate. A missing cell, an unsupported schema, a throughput
drop above the tolerance or a p99 rise above the tolerance fails the gate.
Local execution exercises the mechanism; it is not CI evidence.

## CI workflow

`.github/workflows/bench.yml` runs on `ubuntu-24.04` for `main`, pull requests
and manual dispatches:

1. installs `oha` 1.14.0, nginx and Apache (`apache2`);
2. builds the ExyonQ release binary;
3. runs the harness with `Q12_SERVERS=exyonq,nginx,apache`,
   `3 × 5 s`, 16 connections;
4. uploads `bench-evidence`.

On a pull request, `bench-regression` downloads the latest successful `main`
artifact and runs the gate with 25% RPS / 30% p99 tolerances. Shared GitHub
runners are noisy, so those tolerances are wider than the native local ones.
If no successful `main` artifact exists, the job fails with
`BENCH_BASELINE=NOT_FOUND`; it never invents a baseline or soft-passes.

The first native Linux CI baseline ran on 2026-11-09 as
[run 42695705050](https://github.com/ExyonQ/ExyonQ/actions/runs/42695705050)
on `feat/q12-bench`, commit `5bf2695`, with 3 × 5 s and 16 connections. The
`bench` job succeeded in 9m45s and uploaded `bench-evidence`; the
`bench-regression` job was skipped because the trigger was a push, not a PR.
The numbers are in
[q12-ci-linux-2026-11-09.md](../benchmarks/q12-ci-linux-2026-11-09.md).

The `main` baseline now exists: run `42705724344` at `68d1257`; the PR gate
downloads it and passed on run `42710934813` in normalized mode. A separate
native OLS lane (`.github/workflows/bench-ols.yml`) installs the official
OpenLiteSpeed Ubuntu package and runs the same harness with
`Q12_SERVERS=...,ols`; it is described under
[Native OpenLiteSpeed](#native-openlitespeed). It ran successfully on
2026-11-09 as
[run 42699088690](https://github.com/ExyonQ/ExyonQ/actions/runs/42699088690),
with native OpenLiteSpeed 1.9.4-3+noble; see
[q12-ols-linux-2026-11-09.md](../benchmarks/q12-ols-linux-2026-11-09.md).

## Reproduce

```bash
cargo build --release -p exyonq --bin exyonq --locked
bash scripts/bench/run-http-bench.sh

# shorter local iteration (still needs at least two repetitions for an interval)
Q12_REPETITIONS=3 Q12_DURATION=5s bash scripts/bench/run-http-bench.sh

# subset: only the selected server is required
Q12_SERVERS=exyonq Q12_PAYLOADS=1k.bin bash scripts/bench/run-http-bench.sh
Q12_SERVERS=nginx,apache bash scripts/bench/run-http-bench.sh
```

Requirements: `oha` on `PATH` and only the selected server binaries. On
macOS the system `/usr/sbin/httpd` works; nginx can be installed with
Homebrew. On Debian/Ubuntu set `Q12_HTTPD_BIN=/usr/sbin/apache2`; the module
directory is detected automatically. The output directory is chosen with
`Q12_OUT`.

## Native OpenLiteSpeed

OLS is not launched by the harness because a native install owns its config
and lifecycle. Start OLS on the same host and pass a loopback URL, its PID and
its declared version:

```bash
Q12_SERVERS=exyonq,nginx,apache,ols \
Q12_OLS_URL=http://127.0.0.1:8088 \
Q12_OLS_PID="$(pgrep -f '/usr/local/lsws/bin/lshttpd' | head -n1)" \
Q12_OLS_VERSION="OpenLiteSpeed 1.9.4" \
bash scripts/bench/run-http-bench.sh
```

A non-loopback URL is rejected. A container/VM or remote endpoint is not
comparable even if a host PID is supplied; the operator is responsible for
passing a native process. OLS remains **NOT EXECUTED** on the reference macOS
host because there is no native macOS build there. It **was executed natively
in Linux CI** on 2026-11-09: the job started the distro package from the
tracked `scripts/bench/openlitespeed/` config, disabled gzip, and checked the
sudo-owned process with `ps -p` instead of `kill -0`.

## Executed baseline (2026-11-08)

- Host: Apple M2 Pro, 10 logical CPUs, 32 GiB RAM, macOS 26.5.1 (arm64).
- ExyonQ release `exyonq 0.9.2`, SHA-256
  `f9daa144406415d45b0c031c24b3bc79bb0a24fbafa05a1a7ed9d2d30734f4a9`.
- nginx 1.31.8 (Homebrew), Apache 2.4.66 (system), `oha` 1.14.0.
- 5 repetitions × 10 s, 16 connections; every cell returned HTTP 200 with
  `successRate = 1.0` and passed the SHA-256 body oracle.

| Payload | Server | Median RPS [95% CI] | p99 ms | Peak RSS MiB |
| --- | --- | ---: | ---: | ---: |
| `1k.bin` | ExyonQ | 11804.1 [7050.1, 14384.4] | 12.04 | 16.0 |
| `1k.bin` | nginx | 9305.8 [7670.1, 12711.5] | 22.71 | 21.8 |
| `1k.bin` | Apache | 8309.0 [6216.9, 11716.9] | 25.23 | 14.6 |
| `64k.bin` | ExyonQ | 7378.8 [4286.5, 8360.5] | 18.81 | 18.1 |
| `64k.bin` | nginx | 6915.2 [5411.6, 7256.5] | 18.30 | 21.8 |
| `64k.bin` | Apache | 7197.9 [5608.8, 7273.2] | 16.46 | 14.5 |

The intervals overlap on this host, so the table is an engineering baseline,
not a ranking. The full report, including p50/p95 and every raw repetition,
is [q12-http-native-2026-11-08.md](../benchmarks/q12-http-native-2026-11-08.md);
the JSON next to it is the input for the regression gate.

## Native OLS CI baseline (2026-11-09)

Run `42699088690` measured all four servers on native Ubuntu 24.04 with
OpenLiteSpeed 1.9.4-3+noble, 3 × 5 s and 16 connections. OLS reached
74528.3 RPS at `1k.bin` and 36664.4 at `64k.bin`; ExyonQ in the same run
reached 52217.1 and 43162.5. All 24 raw repetitions had `successRate = 1.0`,
only HTTP 200 responses and zero errors. The complete table, including p99 and
RSS, is in
[q12-ols-linux-2026-11-09.md](../benchmarks/q12-ols-linux-2026-11-09.md). This
is a separate CI run from the branch baseline and is not a gate baseline.

A `main` native OLS baseline now exists: run `42705724338` at `68d1257`
(OpenLiteSpeed 1.9.4-3+noble). On pull requests the OLS workflow runs
`bench-ols-regression`, downloads the latest successful `main` OLS artifact,
requires the `ols` cell and runs the same gate with
`--normalize-to nginx --require-server ols` (25% RPS, 30% p99). A missing
baseline or missing OLS cell fails closed.

## Gate exercise (2026-11-08)

A second real run of the same release binary (3 repetitions × 10 s, same host
and protocol) produced a candidate report. The gate ran with its default
tolerances:

```bash
bash scripts/gates/bench-regression-gate.sh \
  --baseline target/q12-bench-final/report.json \
  --candidate target/q12-bench-candidate/report.json
```

Result: `BENCH_REGRESSION_GATE=PASS`, exit 0, all six cells present. The
candidate was faster than the baseline in every cell, so this exercised the
pass path; it was not a regression run.

The absolute gate was also exercised against the `main` baseline (`68d1257`,
run `42705724344`) with PR #78. It failed because the runs used different EPYC
models: nginx and Apache dropped 23–28% and p99 rose up to 93%, while ExyonQ
stayed inside the 25%/30% limits. Normalized to nginx in the same run,
ExyonQ's RPS ratio moved from 1.129 to 1.115 at `1k.bin` and from 1.103 to
1.221 at `64k.bin`; CI now uses `--normalize-to nginx`.

The failure path was exercised separately with `DEVELOPMENT_AID`: a copy of
the real baseline report with the `1k.bin/exyonq` RPS median halved. The gate
returned exit 1 with
`REGRESSION: 1k.bin/exyonq: rps drop 50.0% > 10.0%`. That is a contract check
of the gate logic, not benchmark evidence.

## Boundaries

- **OpenLiteSpeed is NOT EXECUTED on the reference macOS host.** The harness
  has a native opt-in path, but no native OLS build exists on macOS and Docker
  Desktop is not comparable. The separate Linux CI lane executed native OLS
  in run `42699088690`; that does not establish a macOS build.
- **Linux native is EXECUTED in CI, not on a physical Linux host.** The
  reference workstation is macOS arm64. Run `42695705050` is the first
  `ci_native_github_hosted` baseline on `ubuntu-24.04`; its absolute numbers
  are not portable.
- **The `main` baseline and PR gate are EXECUTED.** `main` is at `68d1257`
  (0.9.2 + Q-12) and run `42705724344` produced the baseline. The first
  absolute PR gate failed on a different runner; CI now uses the normalized
  mode and remains fail-closed if the baseline artifact or a cell is missing.
- **Host noise is real.** On the reference host the 95% intervals of different
  servers can overlap; the report is an engineering baseline, not a ranking.
- The harness does not replace the private official campaign harness.
