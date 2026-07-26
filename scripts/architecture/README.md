# Architecture guardrails (Phase 0)

Tooling-only checks against the kernel freeze contract:

- [`study/reports/11-phase0-kernel-freeze-signoff.md`](../../study/reports/11-phase0-kernel-freeze-signoff.md)
- [`study/reports/KERNEL-AUDIT-current-vs-phase0.md`](../../study/reports/KERNEL-AUDIT-current-vs-phase0.md)

## Scripts

| Script | Purpose |
|--------|---------|
| [`verify-phase0-kernel.sh`](verify-phase0-kernel.sh) | Phase 0 drift: core→mod allowlist, HandlerTable, FastCGI runtime, hot-path forbidden symbols, dispatch vocabulary gaps |
| [`phase0-core-mod-allowlist.txt`](phase0-core-mod-allowlist.txt) | Temporary allowlist until `exyonq-server` composition (PR-7) |

[`verify-oss-boundaries.sh`](../verify-oss-boundaries.sh) delegates to both ADR-026 mod→core checks and Phase 0 kernel lint.

**PR-0 allowlist** (`phase0-core-mod-allowlist.txt`) is **temporary until PR-7** (`exyonq-server` composition). See [ADR-029](../../docs/adr/029-backend-dispatch-phase0.md) § Composition boundary.

## Run

```bash
bash scripts/architecture/verify-phase0-kernel.sh
bash scripts/architecture/verify-phase0-kernel.sh --report study/reports/KERNEL-BOUNDARY-LINT-REPORT.md
EXYONQ_PHASE0_KERNEL_STRICT=1 bash scripts/architecture/verify-phase0-kernel.sh
bash scripts/architecture/verify-phase0-kernel.sh --selftest
bash scripts/verify-oss-boundaries.sh --selftest
```

## Strict mode

- **Violations** (HandlerTable, FastCGI runtime, mod→core, forbidden hot-path): fail when `EXYONQ_PHASE0_KERNEL_STRICT=1`.
- **Warnings** (core→mod outside allowlist): logged; allowlisted paths pass.
- **Info** (EXPECTED-GAP vocabulary): never fails — tracks PR-2/PR-4+ targets.
