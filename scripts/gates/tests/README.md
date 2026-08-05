# Gate tests — benchmark integrity

```bash
bash scripts/gates/benchmark-integrity-gate.sh --selftest
bash scripts/gates/benchmark-integrity-gate.sh --tree .
```

- `--selftest`: fixtures under `fixtures/must-{fail,pass,review}` must behave as named.
- `--tree`: scans product prefixes; **FAIL** on `PRODUCT_SEMANTIC_BRANCH` or `UNKNOWN`.

Known lab state at policy introduction: tree scan **FAIL** while `BENCH_API_CACHE_PATHS` remains in `exyonq-mod-proxy` (see `docs/governance/benchmark-integrity-audit.md`). Do not “fix” the gate by allowlisting that symbol.
