# ExyonQ

ExyonQ is a modular reverse proxy written in Rust.

This repository is a **clean source publication** of the product tree (compile / maintain). Internal benchmark suites, methodology docs, and evidence packs are intentionally not included.

Lab integration tree. Public product release: **v0.4.11** on [exyonq-github](https://github.com/ExyonQ/ExyonQ/releases/tag/v0.4.11) (`ghcr.io/exyonq/exyonq:0.4.11`, no `:latest`). This lab worktree may diverge; do not treat lab HEAD as the published tag.

## Build

```bash
cargo build -p exyonq -p exyonqctl --release --locked
```

```bash
cargo check --workspace --locked
cargo test --workspace --locked
```

Requires a recent Rust toolchain (see `rust-toolchain.toml`) and, for optional HTTP/3 quiche builds, host packages such as `cmake`, `clang`, and `libclang-dev`.

## Defaults

- Allocator: system (optional `allocator-jemalloc` feature on the `exyonq` binary)
- HTTP/3 provider: s2n (optional quiche; quinn-legacy rollback feature)
- Mimalloc is not a product feature

## License

Apache License 2.0 — see `LICENSE` and `NOTICE`.
