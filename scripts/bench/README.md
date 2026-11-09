# Public HTTP benchmark harness (Q-12)

Reproducible HTTP/1.1 benchmark of ExyonQ against selected native servers on
one host. The harness is small on purpose: real servers, real sockets, real
load generator, fail-closed evidence.

```text
PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN unless separately authorized
MEASUREMENT_CLASS       = local_native_non_canonical
```

## What it does

- Starts only the selected native servers on ephemeral loopback ports. All
  read the same payload directory (`benchmarks/scenarios/payloads/www`). The
  default payloads are `1k.bin` and `64k.bin`, both tracked in the repository.
- Verifies every server/payload pair with an exact SHA-256 body oracle before
  measuring.
- Runs interleaved repetitions (default 5) of a closed-loop `oha` load
  (default 16 connections, 10 s), rotating server order per repetition.
  `oha` 1.14 has no thread selector; concurrency is the connection count.
- Samples the resident set of the whole server process tree every 100 ms.
- Rejects any repetition with a non-200 status, success rate below 1.0,
  transport errors or missing RSS evidence.
- Writes raw JSON, `report.json`, `report.md` and the environment metadata
  under the output directory (default: `target/q12-bench-<UTC>`).

## Usage

```bash
cargo build --release -p exyonq --bin exyonq --locked
bash scripts/bench/run-http-bench.sh

# shorter local iteration
Q12_REPETITIONS=3 Q12_DURATION=5s bash scripts/bench/run-http-bench.sh

# subset: only the selected binary is required
Q12_SERVERS=exyonq Q12_PAYLOADS=1k.bin bash scripts/bench/run-http-bench.sh
Q12_SERVERS=nginx,apache bash scripts/bench/run-http-bench.sh
```

Requirements: `oha` on `PATH`, plus only the selected server binaries:

- ExyonQ: `Q12_EXYONQ_BIN` (default `target/release/exyonq`).
- nginx: `Q12_NGINX_BIN` (default `command -v nginx`).
- Apache: `Q12_HTTPD_BIN` (default `httpd` or `apache2`). On Debian/Ubuntu,
  set `Q12_HTTPD_BIN=/usr/sbin/apache2`; the harness auto-detects
  `/usr/lib/apache2/modules` and the other common module directories.
- OpenLiteSpeed: opt-in, see below.

On macOS the system `/usr/sbin/httpd` works; nginx can be installed with
Homebrew. On Linux, `apt-get install nginx apache2 curl` is enough.

## Native OpenLiteSpeed (optional)

OLS is not launched by this harness because a native OLS install owns its own
configuration and lifecycle. Start it yourself on the same host and pass its
loopback endpoint and PID:

```bash
Q12_SERVERS=exyonq,nginx,apache,ols \
Q12_OLS_URL=http://127.0.0.1:8088 \
Q12_OLS_PID="$(pgrep -f '/usr/local/lsws/bin/lshttpd' | head -n1)" \
Q12_OLS_VERSION="OpenLiteSpeed 1.9.4" \
bash scripts/bench/run-http-bench.sh
```

The URL must be loopback (`127.0.0.1`, `localhost` or `[::1]`). The PID must
be alive for the whole run because the harness samples its process tree for
RSS. A container, VM or remote endpoint is **not** comparable: passing a
container's host PID or a remote URL is outside the harness contract even
though a non-loopback URL is rejected outright.

## Anti-regression gate

```bash
bash scripts/gates/bench-regression-gate.sh \
  --baseline baseline/report.json \
  --candidate current/report.json \
  --max-rps-drop-pct 10 --max-p99-rise-pct 15 \
  --normalize-to nginx
```

The gate fails closed when a cell is missing, throughput drops more than the
tolerance or p99 rises more than the tolerance.

With `--normalize-to nginx`, each server's RPS and p99 are divided by the
nginx measurement from the same run before applying the tolerances. This
removes host-wide shifts between the main baseline and a PR candidate;
absolute values remain in every report.

## CI

`.github/workflows/bench.yml` runs the native Linux benchmark on
`ubuntu-24.04` for `main`/PR/manual runs and uploads `bench-evidence`. On a PR,
`bench-regression` downloads the latest successful `main` artifact and runs
the gate against it with wider shared-runner tolerances (25% RPS, 30% p99)
and `--normalize-to nginx`.
There is no fake baseline: if no successful `main` artifact exists, the job
fails with `BENCH_BASELINE=NOT_FOUND`.

The first branch run succeeded on 2026-11-09 (run `42695705050`, Ubuntu
24.04, 3 × 5 s, 16 connections) and its evidence is committed as
`docs/benchmarks/q12-ci-linux-2026-11-09.md`. That was a push run, so the PR
gate was skipped. The `main` baseline is run `42705724344` at `68d1257`; the absolute PR gate
failed on a different EPYC runner, so CI uses the normalized mode for
cross-runner comparisons. The gate remains fail-closed when the reference or
a cell is missing.

On a pull request, `.github/workflows/bench-ols.yml` runs `bench-ols` and then
`bench-ols-regression`. The gate downloads the latest successful native OLS
artifact from `main`, requires the `ols` cell in both reports and runs
`bench-regression-gate.sh --normalize-to nginx --require-server ols` with the
same 25% RPS / 30% p99 shared-runner budget. If no main OLS artifact exists or
the OLS cell is missing, the job fails closed. The first main native OLS
baseline is run `42705724338` at `68d1257`.

`.github/workflows/bench-ols.yml` installs the official native OLS Ubuntu
package and runs `Q12_SERVERS=exyonq,nginx,apache,ols`; it uses the tracked
`scripts/bench/openlitespeed/` config (gzip disabled) and is a separate lane
so it cannot cancel the baseline run. It succeeded on 2026-11-09 as run
`42699088690` with OpenLiteSpeed 1.9.4-3+noble; all 24 raw repetitions had
`successRate = 1.0` and the run ended with `RESULT=PASS Q12_HTTP_BENCH`. See
`docs/benchmarks/q12-ols-linux-2026-11-09.md`.

## Scope and limitations

- **OpenLiteSpeed is opt-in and was not executed on the reference macOS host.**
  There is no native macOS build on that host; Docker Desktop puts it in a
  different kernel/VM, which is not comparable. It was executed natively in
  Linux CI (run `42699088690`); the harness still does not launch OLS itself.
- Numbers are real but **non-canonical**: they are not a competitive claim and
  absolute values are not portable. Reports generated by the CI workflow are
  labelled `ci_native_github_hosted`.
- The harness does not replace the private official campaign harness; it is a
  public, minimal, reproducible engineering baseline.
- Host noise is real. The report is an engineering baseline, not a ranking.
- The branch CI baseline and native Linux OLS lane were executed; a `main`
  baseline and a passing PR gate are still pending because `main` is on 0.9.1.
