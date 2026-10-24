# V044 Core Regression Gate — terminal identity bind

```text
WIP = V044_CORE_REGRESSION_GATE_LINUX_REPEAT_AND_SEAL
CASE = CORE_GATE_LINUX_NETCUP_AMD64_3X_A
DOC_ROLE = TERMINAL_IDENTITY_AND_EVIDENCE_BIND
```

## Bound identities

```text
ENTRY_HEAD = 048afd6eaa56430144ee2f3706f87d7f6402408a
ENTRY_TREE = 9aa366766c9d6b774df612a6152859e2392728a4
SERIES_ID = 20260904T123528Z-netcup-amd64-3x-restart
HOST = netcup-bench
ARCH = x86_64
BINARY_SHA256_AMD64 = 1ce8eab3baa1f0497ce98bcadd1c16699de83c87da72f352af26664082273cd5
```

## Evidence bind

```text
PRIMARY =
  .exyonq-local/evidence/core-regression-gate-linux-repeat-and-seal/20260904T123528Z-netcup-amd64-3x-restart/
AGGREGATE =
  .../summary/aggregate.json
SEAL =
  docs/governance/v044-core-regression-gate-linux-netcup-repeat-seal.md
FREEZE_RECONCILE =
  docs/governance/v044-core-regression-gate-and-stability-freeze.md
ORACLE_COMPLEMENTARY =
  .exyonq-local/evidence/core-regression-gate-and-stability-freeze/20260904T044944Z-coregate/final2-oracle/f2-oracle-1
```

## Green bind

```text
LOCAL_RUN_1 = PASS
LOCAL_RUN_2 = PASS
LOCAL_RUN_3 = PASS
CORE_STATIC = PASS
CORE_PROXY = PASS
CORE_FASTCGI = PASS
CORE_OPERATIONS = PASS
HTTP_OR_TRANSPORT_ERRORS = 0
PROCESS_PANICS = 0
PROCESS_CRASHES = 0
FLAKES = 0
CORE_GATE_GREEN_REPEATABLE = YES
PRODUCT_MUTATION = NO
PHASE_9 = PAUSED
PUBLICATION = NO
```
