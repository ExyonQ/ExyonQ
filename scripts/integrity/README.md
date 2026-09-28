# EXYONQ CODE INTEGRITY AUDITOR

Local, reproducible integrity auditor. It does **not** prove that a feature works
merely because code, tests, config, or docs exist.

```text
POLICY = ZERO_FAKE_REAL_FLOW_ONLY
SAFE_TEST_DOUBLE = NOT_ALLOWED
SMOKE = FORBIDDEN | MOCK = FORBIDDEN | FAKE = FORBIDDEN | STUB = FORBIDDEN

CLAIM != EVIDENCE
CODE_PRESENT != FEATURE_WORKING
TEST_PRESENT != REAL_END_TO_END_BEHAVIOR
CONFIG_ACCEPTED != CONFIG_EFFECTIVE
BENCHMARK_PATH == PRODUCT_PATH
```

Canonical policy: `docs/governance/integrity/zero-fake-policy.md`

## Entrypoint

```bash
./scripts/integrity/exyonq-integrity-audit.sh
./scripts/integrity/exyonq-integrity-audit.sh --quick
./scripts/integrity/exyonq-integrity-audit.sh --changed
./scripts/integrity/exyonq-integrity-audit.sh --selftest
```

If direct `./` execution hangs in a restricted sandbox, invoke via:

```bash
/bin/bash scripts/integrity/exyonq-integrity-audit.sh --selftest
# or
/usr/bin/python3 scripts/integrity/lib/integrity_audit.py selftest --fixtures scripts/integrity/fixtures
```

## Companion: claim/evidence checker

A separate, narrower tool lives beside this auditor:

```bash
./scripts/integrity/claim-evidence-check.sh --selftest
./scripts/integrity/claim-evidence-check.sh --check <file-or-dir>
```

It audits *claims* rather than code: structured `BEGIN_CLAIM_EVIDENCE_BLOCK`
declarations, checked for scope, execution-class, platform, client and
source-state inflation. Policy: `docs/governance/CLAIM_EVIDENCE_INTEGRITY_GUARD.md`.

## Languages

- Bash entrypoint (`set -euo pipefail`, exit codes preserved)
- Python 3 stdlib only (no new Cargo / pip dependencies)

## Artifacts

Default output (worktree-local, not publication surface):

```text
.exyonq-local/integrity/<timestamp>/
  summary.txt
  findings.json
  findings.tsv
  evidence/
.exyonq-local/integrity/latest -> <timestamp>
```

Rationale: repo root `artifacts/` is gitignored generic scratch; `.exyonq-local/`
is the canonical local state root per worktree contract.

Override: `--out DIR` or engine `--out`.

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | PASS — no CRITICAL/HIGH/REVIEW |
| 1 | INTEGRITY_FINDINGS — CRITICAL or HIGH present |
| 2 | AUDITOR_ERROR — engine failure |
| 3 | REVIEW_REQUIRED — REVIEW findings, no CRITICAL/HIGH |

INFO/LOW never alone force non-zero exit.

## Checks

1. Repository boundary  
2. Placeholder / incomplete code  
3. Dead / unreachable path heuristics  
4. Configuration effectiveness  
5. Test integrity (UNIT…REAL_E2E classification)  
6. Hard-coded success / fake results  
7. Error propagation  
8. Feature claim traceability (`docs/governance/integrity/feature-claims.toml`)  
9. Documentation vs code (strong claims → REVIEW)  
10. Security / bypass integrity  
11. Product vs benchmark path  
12. Dependency / feature reality  

## Classifications (ZERO_FAKE)

| Class | Meaning |
|-------|---------|
| REAL_TEST_INPUT | Static known input only (allowed) |
| REAL_COMPONENT / REAL_EXTERNAL_COMPONENT | Reserved for verified real peers |
| FORBIDDEN_SMOKE / _SIMULATION / _MOCK / _FAKE / _STUB / _DUMMY / _PLACEHOLDER / _SHORTCUT | Workspace ban |
| REVIEW_REQUIRED | Ambiguous — human review |

`SAFE_TEST_DOUBLE` is **NOT_ALLOWED**.

## Confidence

Every finding includes `HIGH|MEDIUM|LOW`. Reachability and config-effectiveness
hits are usually `LOW`/`MEDIUM` — the auditor refuses formal proofs it cannot make.

## Feature claims

Edit `docs/governance/integrity/feature-claims.toml`.

Allowed status: `VERIFIED|PARTIAL|UNVERIFIED|DEFERRED|NOT_IMPLEMENTED|BROKEN`.

`VERIFIED` without existing evidence tokens → finding.

## Selftest fixtures

```text
fixtures/must-fail/    → must produce CRITICAL/HIGH
fixtures/must-pass/    → must not produce CRITICAL/HIGH
fixtures/must-review/  → must produce REVIEW/MEDIUM+
```

## What this auditor cannot prove

- Runtime behavior on Netcup/Oracle  
- That a test classified REAL_E2E is dual-arch production evidence  
- Formal dead-code unreachability  
- That CONFIG_CONSUMED implies CONFIG_BEHAVIOR_VERIFIED  
- Semantic truth of documentation beyond lexical strong-claim markers  

When evidence is missing → `UNVERIFIED` / `REVIEW_REQUIRED` / `UNKNOWN`, never fake PASS.
