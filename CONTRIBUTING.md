# Contributing to ExyonQ

Early-stage project. Contributions welcome once public alpha opens (Fase 5).

## Development setup

1. Install Rust 1.93+ (`rust-toolchain.toml` pins the channel).
2. Clone the repo and build:

```bash
cargo build --workspace
cargo test --workspace
```

3. Optional: `cargo install cargo-nextest` for faster test runs (used in CI).

## Before opening a PR

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Or: `cargo run -p xtask -- ci`

Disparar Bugbot antes de merge: comentario `bugbot run` (o `@cursor review`) en el PR. Trigger **manual only** — no asumir revisión automática.

## Scope discipline

- Core changes must respect [ADR-001](docs/adr/001-core-invariants.md).
- Module API changes require ADR amendment and semver bump.
- Compat importers only implement rules listed in [compatibility-scope-v1.md](docs/compat/compatibility-scope-v1.md).
- No P2P, cluster, or panel work in v1 issues unless explicitly un-frozen.

## Local-only paths (do not commit)

Some paths are **local-only** and listed in [`.gitignore`](.gitignore): internal notes, editor workspace config, local analysis output, and internal product or release assessments. See `.gitignore` for the full list.

Before push (or install hooks once):

```bash
bash scripts/verify-no-private-paths.sh
# optional: bash scripts/setup-githooks.sh   # runs the check on every git push
```

Public docs live under `docs/`, `ARCHITECTURE.md`, and ADRs. Repository scope: [`docs/project-scope.md`](docs/project-scope.md).

## Commit style

Imperative subject line, body explains why. Example: `Add spike proxy integration test`.

## License

ExyonQ is licensed under the [Apache License, Version 2.0](LICENSE).

By submitting a contribution to ExyonQ, you agree that your contribution is licensed under the Apache License, Version 2.0. See [NOTICE](NOTICE).
