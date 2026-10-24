# Project Integrity — No Shortcuts, No Fake Data

```text
POLICY_ID = PROJECT-INTEGRITY-NO-SHORTCUTS-NO-FAKE-DATA
POLICY_STATUS = MANDATORY
CURSOR_RULE = .cursor/rules/project-integrity-no-shortcuts-no-fake-data.mdc
PRODUCT_GATE = scripts/gates/project-integrity-gate.sh
DATA_GATE = scripts/gates/data-provenance-gate.sh
RELATED = docs/governance/benchmark-integrity.md
FAIL_MODE = FAIL_CLOSED
```

## Honesty principle (literal)

```text
Nunca alteres el producto, el entorno, los datos o el criterio de evaluación
para producir un resultado más favorable.

Nunca presentes datos simulados, sintéticos, demo, mock, estimados,
inventados o reconstruidos como resultados reales.

Si una medición no existe, debe declararse como ausente.

Si una prueba falla, debe conservarse y reportarse como fallo.

Si el producto no realiza realmente el trabajo que se afirma medir,
el resultado es inválido.
```

## Central chain

```text
REAL_INPUT → REAL_EXECUTION → REAL_OUTPUT → REAL_EVIDENCE
```

Real results require: real input; real execution; real component; declared environment; verifiable identity; reproducible measurement; traceable artifacts.

## Missing is not zero

```text
MISSING != 0
NOT_MEASURED != 0
NOT_APPLICABLE != 0
UNAVAILABLE != 0
FAILED != 0
INVALID != 0
```

Never fill absences with `0`, averages, prior runs, expected, or convenient values. Use explicit states: `MISSING`, `NOT_MEASURED`, `NOT_APPLICABLE`, `NOT_SUPPORTED`, `INVALID`, `FAILED`, `UNAVAILABLE`, `INCONCLUSIVE`.

## Non-real data labels (mandatory)

Any non-real datum must carry one of:

```text
SIMULATED | SYNTHETIC | DEMO | MOCK | ESTIMATED | PROJECTED
EXAMPLE_ONLY | NOT_MEASURED | NOT_REAL_EVIDENCE | NOT_FOR_RANKING | NOT_FOR_RELEASE
```

Estimates additionally require: `VALUE_TYPE=ESTIMATED`, formula/model, assumptions, confidence, and `REAL_MEASUREMENT_AVAILABLE`.

## Product prohibitions

Forbidden in product code: detecting or favoring benchmarks, loadgens, demos, eval envs, known paths/fixtures, scenario names, specific User-Agents, hostnames, ports, `BENCHMARK|DEMO|TEST|PERF` env modes, CI identities, demo modes that change semantics.

Forbidden patterns: hard-coded responses; implicit cache; bypass of upstream/FS/network/auth/protocol; precomputed work for measured paths; error hiding; FAIL→PASS reclassification; artificial work reduction.

Legitimate optimizations must improve normal product use, preserve observable semantics, and not depend on bench/demo detection.

Companion policy for bench-specific incidents: [benchmark-integrity.md](benchmark-integrity.md).

## Mocks and fixtures

Allowed for unit/integration/fault/contract/parser/harness/negative tests and internal demos — **declared as such**.

Forbidden as: production evidence; competitive benchmark; real capacity/perf/scale/availability proof; public claim; release result.

Reports using mocks must show `USES_MOCKS=YES` and `REAL_WORLD_EVIDENCE=NO` unless the mock is the contractual controlled component and documented.

## Demos

Demo mode must never silently change semantics, invent “real” data, hide errors, precompute favorable answers, or write into official result trees. Label: `DEMO_ONLY`, `NOT_REAL_DATA`, `NOT_PRODUCTION_EVIDENCE`, `NOT_FOR_DECISION`.

## Identity and provenance

Every real result records: source commit, branch, dirty state, binary hash, image digest, version, config hash, fixture hash, host, arch, tool identity/version, command, timestamp, run ID, environment, resource limits.

Missing critical identity → `RESULT_STATUS=INVALID_IDENTITY`. No reuse across distinct identities.

Pipeline: `raw real → validation → normalization → aggregation → reconciliation → report`. Manual metric edits, fabricated raw/logs/seals/hashes, cherry-picking best runs, or rewriting historical evidence are forbidden.

## Causality

Measured component must actually do the work (proxy without explicit cache: N client ≈ N upstream; FS measurements cannot serve embedded bodies; external API measurements cannot silently stub; etc.).

## Differential rule

```text
NORMAL_EXECUTION_BEHAVIOR = BENCHMARK_EXECUTION_BEHAVIOR = TEST_EXECUTION_BEHAVIOR
```

except observational variance (logs, tracing, profiling, metric capture).

## Agent declaration

Before perf/test/bench/demo/measurement work, declare the fields in the Cursor rule. Real-evidence claims require real data YES and all shortcut/fake fields NO. Else `OWNER_REVIEW_REQUIRED=YES`, `EXECUTION_AUTHORIZED=NO`.

## Process gates

| Phase | Requirement |
|-------|-------------|
| Pre-test / pre-demo / pre-bench | `project-integrity-gate` selftest; tree PASS for product claims |
| Pre-competitive run | Also `benchmark-integrity-gate` PASS |
| Pre-publication | `data-provenance-gate` PASS on the result package |
| Pre-release | Both gates PASS; zero `PRODUCT_SEMANTIC_BRANCH`, `FAKE_RESULT_GENERATION`, `UNKNOWN` |
| Local / architecture / security review | Include integrity checklist |

Concealment (scenario swaps, mock substitution, criterion lowering, evidence deletion, filling gaps, mixing runs, demo-as-prod) is forbidden. On violation: stop, preserve evidence, root-cause, notify owner, separate real vs simulated, authorize fix, re-validate only the affected phase.

## Remediation status (Phase 1)

```text
REMEDIATION_DOC = docs/governance/project-integrity-remediation-phase-1.md
AUDIT = docs/governance/project-integrity-audit.md
P4_REVERIFY = docs/benchmarks/bv04-p4-integrity-remediation-reverify.md
SEALS = benchmarks/bv04/seals/P4_INTEGRITY_REMEDIATION_LATEST.json
        benchmarks/bv04/seals/P4_INTEGRITY_FIXTURE_REVERIFY_LATEST.json
```

Critical/High product shortcuts (`BENCH_API_CACHE_PATHS`, force/reuse `api_cache`, `EXYONQ_BENCH_CACHE_HEADERS` semantics) remediated under owner authorization on 2026-07-31. `BENCH_SMALL_UPSTREAM_BODY` remains a **uniform per-response materialize threshold** (not a cross-request cache); scanners must not classify the symbol name alone as `PRODUCT_SEMANTIC_BRANCH` (GRC-R5D refinement). Residual REVIEW (`BENCH_HEADER_*` / `P1_BENCH_WIRE_RODATA`) remains classify-only until a later phase. Historical failure evidence is retained in the audit; do not overwrite.

## Commands

```bash
bash scripts/gates/project-integrity-gate.sh --selftest
bash scripts/gates/project-integrity-gate.sh --tree .
bash scripts/gates/data-provenance-gate.sh --selftest
bash scripts/gates/data-provenance-gate.sh --check benchmarks/results/<run-id>
```
