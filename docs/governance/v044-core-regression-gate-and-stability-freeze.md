# V044 Core Regression Gate & Stability Freeze

```text
WIP = V044_CORE_REGRESSION_GATE_AND_STABILITY_FREEZE
SUCCESSOR_WIP = V044_CORE_REGRESSION_GATE_LINUX_REPEAT_AND_SEAL
STATUS = CORE_GATE_GREEN_REPEATABLE
PUBLICATION = NO
PRODUCT_MUTATION = NO
PHASE_9 = PAUSED
NEW_PRODUCT_CAPABILITIES = PAUSED_UNTIL_OWNER_AUTHORIZES_NEXT
NEXT_CAPABILITY_AUTHORIZED = NO
```

## Purpose

One top-level, repeatable gate proving that supported **static**, **reverse proxy**, **FastCGI**, and **basic operations** still work together after a change. This is a stability freeze tool — not a new product capability and not a benchmark.

## Top-level command

```bash
bash scripts/gates/core-regression-gate.sh \
  --evidence-root .exyonq-local/evidence/core-regression-gate-and-stability-freeze/<RUN_ID> \
  --run-id <RUN_ID>
```

Prerequisites (Linux):

- buildable `exyonq` / `exyonqctl`
- `curl`, `python3`, `sha256sum`
- `php-fpm` for FastCGI axes
- unique temp ports; runner owns cleanup of processes it starts
- AF_UNIX sockets under short `/tmp/exyonq-cg-*.sock` paths (deep evidence paths exceed sockaddr limits)

Darwin/macOS: authoritative e2e scripts call `e2e_require_linux` and exit `NOT_EXECUTED`. Darwin-local consecutive green is **not** required after owner authorization that Netcup amd64 3× counts as the required local series.

## Axes and oracles

| Axis | Cases | Oracle (must fail on timeout) |
| --- | --- | --- |
| CORE_STATIC | `scripts/e2e/static-e2e.sh` | GET/HEAD; SHA256 for 1KiB, 64KiB, **1MiB**; traversal + symlink escape reject; progress after reject |
| CORE_PROXY | `scripts/e2e/proxy-e2e.sh` → `suites/proxy-suite.sh` | healthy upstream; down→502; timeout→502/504; reload identity; **1MiB body SHA256**; HTTP/1.1 KA two-request **or** Cap034 documented single-request/`Connection: close` lifecycle |
| CORE_FASTCGI | `fastcgi-php-fpm-e2e.sh` + `03b-wordpress-profile.sh` | real PHP-FPM GET/POST/502/503/504; WP-profile front + wp-admin PHP (docroot + case dir must be traversable by `www-data`) |
| CORE_OPERATIONS | `04-operations.sh` | start+readiness; ctl reload without panic; clean stop; no unreaped server leak |

Evidence per run:

```text
<evidence-root>/
  meta/run.env
  cases/<axis>_<name>/{case.env,stdout.txt,stderr.txt}
  summary/{axes.env,result.env,summary.json,console.log}
  cleanup/process-ledger.txt
```

## Repeatability result (authoritative)

### Required local series (owner-authorized Netcup amd64 3×)

```text
SERIES = 20260904T123528Z-netcup-amd64-3x-restart
EVIDENCE = .exyonq-local/evidence/core-regression-gate-linux-repeat-and-seal/20260904T123528Z-netcup-amd64-3x-restart/
SEAL_DOC = docs/governance/v044-core-regression-gate-linux-netcup-repeat-seal.md
LOCAL_RUN_1 = PASS
LOCAL_RUN_2 = PASS
LOCAL_RUN_3 = PASS
CORE_GATE_GREEN_REPEATABLE = YES
```

### Complementary dual-arch history (same HEAD/TREE)

Prior package (identity-compatible; Oracle not re-run under Linux-repeat WIP):

```text
.exyonq-local/evidence/core-regression-gate-and-stability-freeze/20260904T044944Z-coregate/
```

| Platform | Result | Notes |
| --- | --- | --- |
| Darwin local | `NOT_EXECUTED` | `e2e_require_linux`; not required for green after Netcup-3× authorization |
| Netcup amd64 (prior final2) | **3/3 PASS** | `final2-netcup/f2-netcup-{1,2,3}` |
| Netcup amd64 (this seal) | **3/3 PASS** | authoritative required series |
| Oracle arm64 | **1/1 PASS** | `final2-oracle/f2-oracle-1` complementary |

Binary SHA256 (candidate harness tree under `/tmp/exyonq-coregate-048afd6e`):

```text
amd64 = 1ce8eab3baa1f0497ce98bcadd1c16699de83c87da72f352af26664082273cd5
arm64 = 7bbcf6c87676a381a16c377674c250e53e00a2577d298f6b1c8d54d69daa99bc
```

Source identity:

```text
HEAD = 048afd6eaa56430144ee2f3706f87d7f6402408a
TREE = 9aa366766c9d6b774df612a6152859e2392728a4
```

## Harness-only notes (not product defects)

1. WordPress profile 404 body `File not found.` was php-fpm unable to traverse `mktemp` `0700` case dir — fixed with `chmod a+rwX "$CASE_DIR"` (same as FastCGI suite).
2. Proxy keep-alive oracle accepts Cap034 documented single-request lifecycle.
3. Control sockets namespaced under `/tmp` to avoid AF_UNIX path length and host socket collisions.

`SUPPORTED_PRODUCT_DEFECT_FOUND = NO` under this authorization (no product repair performed).

## Freeze / pause (still in force)

```text
PHASE_8 = REMAINS_CLOSED
NO_NEW_STATIC_FEATURES = YES
NO_NEW_PROXY_FEATURES = YES
NO_NEW_FASTCGI_FEATURES = YES
NO_BENCHMARK_EXECUTION = YES
NO_HAPROXY_WORK = YES
NO_PERFORMANCE_TUNING = YES
NO_PRODUCT_REPAIR = YES
PHASE_9 = PAUSED
FIVE_AXIS_BENCHMARK = DIAGNOSTIC_ONLY
CPU_RSS = NON_COMPARABLE_NOT_RANKABLE
PUSH = NO
PUBLICATION = NO
```

Canonical integration of these documentation commits requires explicit owner approval after FF safety review.
