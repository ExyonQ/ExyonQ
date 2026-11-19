# Config compatibility promise V1

**Status:** active for the ExyonQ 0.9.x line, from 0.9.2 onward
**Surface:** the native IR configuration (`config.toml` / `config_version`), read at load time by `exyonq` and by `exyonqctl config lint|migrate`
**Out of scope:** config importers — see [compatibility-scope-v1.md](compatibility-scope-v1.md)

This is the config half of the `0.9.2` Definition of Done item "stable release
with the config compatibility promise". It states what an operator may rely on
when moving between 0.9.x releases, and what is deliberately not covered.

## Promised

1. **A config that validates on a published 0.9.x release keeps validating on
   every later 0.9.x release**, unless the exception below applies.
   *Enforced:* `scripts/gates/config-compat-promise-gate.sh` validates the real
   published example of the previous public release
   (`scripts/gates/fixtures/config-compat/v0.9.1-config.toml`, copied verbatim
   from tag `v0.9.1`) with the current binary, and fails the build when it no
   longer validates.
2. **`config_version` only moves for a breaking change**, and a new version
   ships with its migration. Today `1` and `2` are accepted; `2` adds `include`
   fragments, and `exyonqctl config migrate` rewrites a v1 file textually
   (only the version line) and then re-parses the result before writing it. See
   [070-config-version-migration.md](../architecture/070-config-version-migration.md).
3. **Unknown keys are rejected, never ignored.** Every config struct is
   `deny_unknown_fields`, so a typo, a key from a newer release or a key that
   was removed fails at load with the field named (`EXY-CONFIG-0002`). This is
   fail-closed on purpose: a silently ignored directive is a security defect,
   not a convenience.
4. **Published examples validate and stay on the current version**:
   `scripts/gates/config-examples-validate.sh` validates them with the real
   binary and `scripts/gates/config-migrate-check.sh` fails when one drifts to
   an older `config_version`.

## Not promised

- nginx, Apache or OpenLiteSpeed import parity; `TOTAL_COMPAT_PROMISE` stays
  **FORBIDDEN** for those sources until their gate corpus passes on the native
  matrix.
- Undocumented keys, CLI output formats, diagnostic wording and internal
  identifier names.
- A migration across a major version bump (0.x → 1.0) or a downgrade command.
- Configurations that never validated on any published release. The 0.9.0
  example used a `metrics_path` key the IR never accepted; that documentation
  drift was fixed under EXY-13 and is not a compatibility obligation.

## Allowed exception

A later 0.9.x release may tighten a **validation** rule for a security reason.
When it does, the release notes must list the rule, its error code and what an
operator has to change; the config then fails closed with an actionable message
instead of being silently rewritten. 0.9.2's per-route `max_request_body_bytes`
budget (`413` before inspection, with unset routes keeping the 64 MiB contract)
is documented in [docs/release/v0.9.2.md](../release/v0.9.2.md).

## Verification

| Check | Command |
| --- | --- |
| Previous published config still validates | `bash scripts/gates/config-compat-promise-gate.sh` |
| Published examples validate | `bash scripts/gates/config-examples-validate.sh` |
| Published examples are current | `bash scripts/gates/config-migrate-check.sh` |

The first gate also runs a self-test that appends an unknown key to the fixture
and requires a non-zero exit, so a green run cannot come from a validator that
accepts everything.
