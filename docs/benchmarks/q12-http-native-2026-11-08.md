# Q-12 HTTP/1.1 benchmark — native local baseline

> `measurement_class = local_native_non_canonical`. These numbers are real
> executions on the declared host, but they are **not** canonical Linux
> evidence and must not be used for competitive or release claims.

## Environment

- Host: Apple M2 Pro, 10 logical CPUs, 32 GiB RAM
- OS: macOS 26.5.1 (arm64)
- ExyonQ: `exyonq 0.9.2` (SHA-256 `f9daa144406415d45b0c031c24b3bc79bb0a24fbafa05a1a7ed9d2d30734f4a9`)
- nginx: `nginx version: nginx/1.31.8`
- Apache: `Server version: Apache/2.4.66 (Unix)`
- Load generator: `oha 1.14.0`
- Repetitions: 5 × 10s, 16 connections; `oha` 1.14 has no thread selector
- Source: `bfdb22ebeff31226157d3bdc4237994edf1d02ea` (working tree dirty: yes)

## Results

Values are median over repetitions with a 95% bootstrap interval of the
median (10,000 resamples). RSS is the peak resident set of the server
process tree sampled every 100 ms.

### `1k.bin`

| Server | Requests/s | p50 ms | p95 ms | p99 ms | RSS peak MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| exyonq | 11804.1 [7050.1, 14384.4] | 0.86 [0.80, 1.23] | 2.93 [2.02, 5.61] | 12.04 [7.19, 24.28] | 16.0 [15.3, 17.8] |
| nginx | 9305.8 [7670.1, 12711.5] | 1.00 [0.63, 1.04] | 2.85 [1.88, 4.55] | 22.71 [7.50, 35.70] | 21.8 [21.6, 23.9] |
| apache | 8309.0 [6216.9, 11716.9] | 1.01 [0.73, 1.14] | 4.40 [2.42, 6.90] | 25.23 [7.89, 32.36] | 14.6 [14.1, 15.2] |

### `64k.bin`

| Server | Requests/s | p50 ms | p95 ms | p99 ms | RSS peak MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| exyonq | 7378.8 [4286.5, 8360.5] | 1.62 [1.54, 2.23] | 4.72 [3.31, 10.52] | 18.81 [12.26, 29.38] | 18.1 [17.6, 18.9] |
| nginx | 6915.2 [5411.6, 7256.5] | 1.84 [1.40, 1.89] | 3.89 [3.58, 5.09] | 18.30 [15.33, 40.89] | 21.8 [19.0, 22.0] |
| apache | 7197.9 [5608.8, 7273.2] | 1.83 [1.59, 1.95] | 3.68 [3.51, 7.32] | 16.46 [16.15, 29.73] | 14.5 [14.3, 15.3] |

## Method

- All servers run natively on the same host and kernel, read the same
  payload files from one directory, use HTTP/1.1 keep-alive and disable
  access logging.
- Warm-up: 2 s per server/payload before the measured repetitions.
- Each repetition starts a fresh 10s closed-loop `oha` run; server order
  rotates per repetition to spread drift.
- Every response is validated by `oha` (status 200, success rate 1.0, no
  errors) and, before measurement, by an exact SHA-256 body oracle per
  server/payload.

## Limitations

- Native macOS arm64 with Homebrew nginx and the system Apache; this is
  not the canonical Linux matrix and absolute numbers are not portable.
- The historical JSON contains `threads: 4`; that field was a declaration
  error, not an `oha` setting. `oha` 1.14 has no thread selector; the
  actual concurrency is the 16 declared connections.
- OpenLiteSpeed is **NOT EXECUTED** on this host: there is no native macOS
  build, and the Docker image would run in a different kernel/VM, which is
  not comparable. The harness now supports a native OLS process via a
  loopback URL/PID; it was not used for this baseline.
- CI execution is **NOT EXECUTED**: `bench.yml` and the baseline gate are
  implemented, but no push or first `main` run exists yet.
