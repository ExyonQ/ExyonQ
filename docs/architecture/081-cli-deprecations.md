# CLI deprecations

ExyonQ keeps the historical single-binary subcommands working while the control
CLI (`exyonqctl`) becomes the supported surface. This page states the timeline
so operators can plan; the binaries print the same information.

## Deprecated in 0.9.2, migration completed in 0.10.0, removed in 0.11.0

| Transitional command | Replacement | Notes |
| --- | --- | --- |
| `exyonq validate <path>` | `exyonqctl config lint <path>` | parse + validate only; no RuntimePlan |
| `exyonq validate <path>` (compile check) | `exyonqctl config test <path>` | compiles the RuntimePlan as well |
| `exyonq fmt <path>` | `exyonqctl config format <path>` | same canonical TOML output |
| `exyonq explain` | `exyonqctl config explain` | effective values, not defaults |
| `exyonq compile` | `exyonqctl config test` | product surface is the control CLI |

`exyonq validate` prints a warning that names the replacement for the exact file
you passed, and still exits with the validation result:

```text
warning: `exyonq validate /etc/exyonq/sites.toml` is transitional and is removed in 0.11.0; use `exyonqctl config lint /etc/exyonq/sites.toml` (or `exyonqctl config test /etc/exyonq/sites.toml` for RuntimePlan compile). See docs/architecture/081-cli-deprecations.md
OK
```

## What to change in a deployment

1. Replace `exyonq validate ...` in unit files, installers and runbooks with
   `exyonqctl config lint ...` (or `config test` where the RuntimePlan matters).
2. Keep passing the configuration path positionally; `exyonqctl config lint`
   also accepts `--config`.
3. Do not rely on the transitional subcommands after 0.11.0; there is no
   fallback.

The warning is the source of truth for the current release; this page is updated
with it.
