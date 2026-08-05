# Security review checklist

Structured guide for manual security reviews of ExyonQ core and official modules. Used for release audits ([`audit-template.md`](audit-template.md)) and ad-hoc reviews.

## Scope

Treat each component as Internet-exposed network software. Assume hostile input on every parser, header, path, and upstream response boundary.

## Priority surfaces

Review with highest priority:

- HTTP parsers, framing, headers, chunking, compression, and derived protocols.
- Timeouts, cancellation, retries, pools, queues, backpressure, and resource limits.
- `unsafe` code, FFI, C/C++ bindings, syscalls, and manual buffer handling.
- Hot reload, config parsing, config merges, and implicit defaults.
- Path, header, host, pseudo-header, and hop-by-hop header normalization.
- Logs, metrics, tracing, and accidental secret exposure.

## Threat patterns

Look actively for:

- Request smuggling and parsing ambiguity.
- Integer overflow/underflow and unbounded allocation.
- Memory unsafety or broken invariants in `unsafe`.
- Deadlocks, starvation, data races, and incomplete cancellation.
- Path traversal and normalization bugs.
- Panic paths reachable from external input.
- Frontend parser vs upstream semantic mismatches.
- Authentication, ACL, or tenant isolation bypasses (when applicable).

## Finding format

For each finding, record:

1. Severity: critical, high, medium, or low.
2. File and line.
3. Affected function or module.
4. Trigger condition.
5. Technical impact.
6. Reproduction steps or test design.
7. Minimal patch suggestion.
8. Required regression test.

If evidence is insufficient, mark the item as **hypothesis** and note the missing test.

## Change constraints

- Avoid cosmetic refactors in security-fix PRs.
- Do not change public names or APIs without impact justification.
- Do not add dependencies without clear need and rationale.
- Do not trade safety for speed without documented invariants and risks.
- When touching `unsafe`, document preconditions and invariants in code comments.

## Recommended follow-ups

- Unit, property, and regression tests for each fix.
- Fuzz harnesses (`cargo-fuzz`) for parsers and framing.
- Explicit size, time, and concurrency limits.
- Replace `unwrap()` / `expect()` on untrusted paths with controlled errors.
- Miri, sanitizers, and differential testing where applicable.

## Exit criteria

Do not close a review with vague conclusions. If no issues are found, list surfaces reviewed, unvalidated assumptions, and missing automated tests that would increase confidence.
