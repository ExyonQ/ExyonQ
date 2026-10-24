# V044 Core Regression Gate — Linux Netcup amd64 3× repeat seal

```text
WIP = V044_CORE_REGRESSION_GATE_LINUX_REPEAT_AND_SEAL
CASE = CORE_GATE_LINUX_NETCUP_AMD64_3X_A
STATUS = CORE_GATE_GREEN_REPEATABLE
VERDICT = PASS
PUBLICATION = NO
PRODUCT_MUTATION = NO
HARNESS_COMMITTED = NO
PHASE_9 = PAUSED
BENCHMARK_EXECUTION = NO
PUSH = NO
TAG = NO
RELEASE = NO
CANONICAL_MERGE = PENDING_OWNER_REVIEW
```

## Owner authorization applied

```text
NETCUP_AMD64_3X_COUNTS_AS_REQUIRED_LOCAL_SERIES = YES
DARWIN_LOCAL_3X_REQUIRED = NO
ORACLE_ARM64_1X = COMPLEMENTARY_NO_REPEAT_UNLESS_FAIL
```

## Source identity

```text
ENTRY_HEAD = 048afd6eaa56430144ee2f3706f87d7f6402408a
ENTRY_TREE = 9aa366766c9d6b774df612a6152859e2392728a4
BINARY_SHA256_AMD64 = 1ce8eab3baa1f0497ce98bcadd1c16699de83c87da72f352af26664082273cd5
REMOTE_ROOT = /tmp/exyonq-coregate-048afd6e
HOST = netcup-bench (Linux x86_64)
```

## Authoritative series (this WIP)

```text
SERIES_ID = 20260904T123528Z-netcup-amd64-3x-restart
EVIDENCE =
  .exyonq-local/evidence/core-regression-gate-linux-repeat-and-seal/20260904T123528Z-netcup-amd64-3x-restart/
```

| Run | CORE_STATIC | CORE_PROXY | CORE_FASTCGI | CORE_OPERATIONS | RESULT |
|-----|-------------|------------|--------------|-----------------|--------|
| LOCAL_RUN_1 (`linux-netcup-1`) | PASS | PASS | PASS | PASS | PASS |
| LOCAL_RUN_2 (`linux-netcup-2`) | PASS | PASS | PASS | PASS | PASS |
| LOCAL_RUN_3 (`linux-netcup-3`) | PASS | PASS | PASS | PASS | PASS |

```text
HTTP_OR_TRANSPORT_ERRORS = 0
PROCESS_PANICS = 0
PROCESS_CRASHES = 0
FLAKES = 0
FAILURE_ATTRIBUTION = NONE
CORE_GATE_GREEN_REPEATABLE = YES
```

Invalidated non-counting attempt (scanner false positive on prose `without panic`):

```text
.exyonq-local/evidence/core-regression-gate-linux-repeat-and-seal/20260904T123339Z-netcup-amd64-3x/invalidated-scanner-false-positive/
```

## Complementary Oracle arm64

Prior identity-compatible series (not re-run):

```text
.exyonq-local/evidence/core-regression-gate-and-stability-freeze/20260904T044944Z-coregate/final2-oracle/f2-oracle-1
ORACLE_ARM64 = PASS_1OF1
BINARY_SHA256_ARM64 = 7bbcf6c87676a381a16c377674c250e53e00a2577d298f6b1c8d54d69daa99bc
```

## Regression confirmation

Under this series, supported **Static**, **Reverse proxy**, **FastCGI**, and **Operations** paths did **not** regress: all four axes PASS on each of three consecutive Netcup amd64 executions.

## Scope limits (explicit)

```text
PHASE_8 = REMAINS_CLOSED
PHASE_9 = PAUSED
OFFICIAL_BENCHMARKING = PAUSED
FIVE_AXIS_BENCHMARK = DIAGNOSTIC_ONLY_IF_PRESENT
RPS_LATENCY = QUALIFIED_ONLY_WHEN_SEPARATELY_AUTHORIZED
CPU_RSS = NON_COMPARABLE_NOT_RANKABLE
NO_PRODUCT_PERFORMANCE_CONCLUSION = YES
NO_PUBLICATION = YES
NEXT_CAPABILITY_AUTHORIZED = NO
```

## Harness note

Gate harness scripts used for execution remain **uncommitted** under this seal (docs-only commits). Product source was not mutated.
