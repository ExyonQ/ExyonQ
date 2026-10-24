# Data Provenance Integrity

```text
POLICY_ID = EXYONQ-DATA-PROVENANCE-INTEGRITY
POLICY_STATUS = MANDATORY
CURSOR_RULE = .cursor/rules/project-integrity-no-shortcuts-no-fake-data.mdc
GATE = scripts/gates/data-provenance-gate.sh
RELATED = docs/governance/project-integrity.md
FAIL_MODE = FAIL_CLOSED
```

## Purpose

A result package may be published, ranked, or used as release evidence only when
identity, raw artifacts, and labels are complete and honest.

## Required package fields

At minimum a checkable package under `--check DIR` must include:

| Artifact | Role |
|----------|------|
| `run_meta.json` (or equivalent identity file) | commit/revision, command, host/arch, timestamp/run ID |
| `raw/` (or declared raw evidence) | unmodified measurement outputs |
| metrics / summary files | derived from raw only |

## Absolute bans

```text
MISSING != 0
NOT_MEASURED != 0
DEMO/MOCK/SYNTHETIC presented as official = FAIL
manual metric edits / fabricated seals = FAIL
```

Non-real data must carry an explicit label from the project-integrity policy.

## Commands

```bash
bash scripts/gates/data-provenance-gate.sh --selftest
bash scripts/gates/data-provenance-gate.sh --check DIR
```

```text
DATA_PROVENANCE_GATE != PASS → no publish / no ranking / no release evidence claim
```
