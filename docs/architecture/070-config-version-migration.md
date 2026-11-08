# Config version migration (Q-11)

The IR is versioned through the top-level `config_version` key. Version 1 is the
original format; version 2 adds `include` fragments. `exyonqctl config migrate`
moves an existing v1 file to v2 without rewriting anything else.

## Command

```bash
exyonqctl config migrate --config /etc/exyonq/config.toml
```

- Rewrites in place and keeps `/etc/exyonq/config.toml.v1.bak`.
- **Only the `config_version` line changes.** Comments, key order, quoting and
  every value stay byte-for-byte identical. Migration is textual on purpose:
  parsing and re-rendering the IR would drop comments and formatting, so the
  command rewrites the one line, then parses the result to prove it is a valid
  `config_version = 2` document before writing it.
- `--output <path>` writes a new file and leaves the source untouched.
- `--check` writes nothing and exits `1` while the file is still v1. The
  `config-migrate-check` CI job runs it over the published examples
  (`scripts/gates/config-migrate-check.sh`) and fails on drift; once the file
  is v2 it prints `MIGRATION=NOT_NEEDED` and exits `0`.
- `--no-backup` skips the `.v1.bak` copy. The command refuses to overwrite an
  existing backup: remove it deliberately or pass `--no-backup`.

Exit codes: `0` migrated or already current, `1` invalid config or blocked
backup, `2` unreadable input or write failure.

## Rollback

1. Stop the service: `systemctl stop exyonq`.
2. Restore the backup: `mv /etc/exyonq/config.toml.v1.bak /etc/exyonq/config.toml`.
3. Validate: `exyonq validate --config /etc/exyonq/config.toml` must pass.
4. Start the service and check `exyonqctl status`.

Rollback is safe because v2 only enables `include`; a file that never used
`include` is valid under both versions. There is no downgrade command: restoring
the backup is the supported path.

## Guarantees and limits

- `config_version` 1 and 2 are the only accepted values; anything else fails
  validation as an unsupported version.
- A v1 file that already contains `include` is rejected by the parser, because
  `include` requires version 2. Fix the directive before migrating.
- The command never talks to a running server, never reloads it and never
  publishes a RuntimePlan.
- Future versions need their own migration step; this command only implements
  v1 -> v2.

## Verification

- `cli/exyonqctl/tests/config_migrate.rs` runs the real binary over: `--check`
  reporting drift without writing, an in-place rewrite whose diff is exactly one
  line plus a `.v1.bak` copy, `--output` leaving the source intact, and an
  invalid config rejected without touching the file.
- `exyonqctl config lint <migrated>` must pass after the rewrite.
