# ExyonQ

ExyonQ is a modular reverse proxy written in Rust.

This repository is a **clean source publication** of the product tree (compile / maintain). Internal benchmark suites, methodology docs, and evidence packs are intentionally not included.

Current product version: **v0.4.9**. Release: https://github.com/ExyonQ/ExyonQ/releases/tag/v0.4.9. No `:latest` tag.

## Container images

Public. No GitHub login.

GitHub Container Registry:

```bash
docker pull ghcr.io/exyonq/exyonq:0.4.9
docker pull ghcr.io/exyonq/exyonq-wordpress:0.4.9
```

Docker Hub:

```bash
docker pull exyonq/exyonq:0.4.9
docker pull exyonq/exyonq-wordpress:0.4.9
```

`exyonq` is the server image. `exyonq-wordpress` is the WordPress image. Both are `linux/amd64` and `linux/arm64`. The release also has the Linux binaries.

https://hub.docker.com/r/exyonq/exyonq
https://hub.docker.com/r/exyonq/exyonq-wordpress

https://github.com/ExyonQ/ExyonQ/packages

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

## Contact

- contact@exyonq.org
- security@exyonq.org

## License

Apache License 2.0 — see `LICENSE` and `NOTICE`.
