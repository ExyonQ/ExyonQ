# ADR-053 — Apache `Include` / `IncludeOptional` (AP-07 / v0.5 F1)

**Status:** Accepted  
**Date:** 2026-10-18  
**Depends on:** Apache importer allowlist (rewrite / ErrorDocument), nginx `compat/nginx` include loader (limits mirror)  
**Contract:** CONDITIONAL ACCEPT (`exyonq-architecture-contract` — checklist in this ADR)

## Context

Apache configs almost always split via `Include` / `IncludeOptional`. COMPAT_APACHE previously required operators to pass every fragment as extra `--input`. Roadmap AP-07 requires nested includes from the corpus to **migrate or reject with `EXY-*` + `file:line`**, never silently.

Nginx Full-profile path migrate already expands `include` (`compat/nginx/src/include.rs`). Apache string-only migrate did not.

## Decision

### Layer

Offline only in `exyonq-compat-apache` (+ `exyonq-compat` CLI path entry). No runtime / IR `include` / hot-path.

### API

| Entry | Includes |
| --- | --- |
| `migrate` / `migrate_with_report(label, &str)` | **No disk expand.** Encountering `Include`/`IncludeOptional` → `REJECTED_UNSUPPORTED` with `EXY-AP-INCLUDE-0001` |
| `migrate_path` / `migrate_path_with_report(&Path)` | Expand relative to including file; fail-closed |

### Semantics

| Directive | Missing / empty glob | Found |
| --- | --- | --- |
| `Include` | Reject `EXY-AP-INCLUDE-0002` | Inline-parse into same `ParseState` |
| `IncludeOptional` | Compat note (not fatal) | Same as Include |

- **Base directory:** parent of the **including** file (not ServerRoot emulation).
- **Absolute paths:** allowed only if canonical path stays under **entry root** (canonical parent of the entry file). Otherwise `EXY-AP-INCLUDE-0005`.
- **Globs:** single `*` segment in the last path component (e.g. `conf.d/*.conf`), deterministic sort.
- **Cycles / depth / budgets:** mirror nginx limits — depth ≤ 16, ≤ 1024 files, ≤ 16 MiB total bytes (`EXY-AP-INCLUDE-0003`…`0007`).

### Codes

| Code | Meaning |
| --- | --- |
| `EXY-AP-INCLUDE-0001` | Include seen without path-based migrate |
| `EXY-AP-INCLUDE-0002` | Required Include matched no files / not found |
| `EXY-AP-INCLUDE-0003` | Include cycle |
| `EXY-AP-INCLUDE-0004` | Include depth exceeded |
| `EXY-AP-INCLUDE-0005` | Path escapes entry root / traversal / unsafe absolute |
| `EXY-AP-INCLUDE-0006` | IncludeOptional matched nothing (note only) |
| `EXY-AP-INCLUDE-0007` | File count or byte budget exceeded |

Every reject line format: `REJECTED_UNSUPPORTED {file}:{line} \`Include\` — EXY-AP-INCLUDE-NNNN: …`.

### Non-goals

- Emulating Apache `ServerRoot` / `IncludeOptional` wildcards beyond one `*`
- Runtime config includes
- Q-02 serve-path diagnostics

## Consequences

- `exyonq-compat --from apache --input httpd.conf` expands nested Includes.
- COMPAT_APACHE §6 drops “pass every Include as extra `--input`” for AP-07-covered cases.
- Nginx CLI should use `migrate_file` for Full profile so corpus `include` is not silent.
