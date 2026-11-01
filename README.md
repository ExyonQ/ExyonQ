# ExyonQ

ExyonQ is a modular HTTP server and reverse proxy written in Rust: static files, FastCGI/PHP-FPM, TLS, HTTP/2 and HTTP/3, full page cache, WAF, and a balancer with nginx/Apache-compatible configuration migration.

ExyonQ includes migration tools and aims to progressively expand compatibility with Apache, NGINX, and OpenLiteSpeed configurations. Refer to the documentation for details on the scope of each importer.

This repository is a **clean source publication** of the product tree (compile / maintain). Internal benchmark campaigns and raw evidence packs are intentionally not included; methodology notes and summary dashboards live under `docs/benchmarks/`.

Lab integration tree. Public product release: **v0.8.0** on [exyonq-github](https://github.com/ExyonQ/ExyonQ/releases/tag/v0.8.0) (`ghcr.io/exyonq/exyonq:0.8.0`, `ghcr.io/exyonq/exyonq-wordpress:0.8.0`, `exyonq/exyonq:0.8.0` on Docker Hub — no `:latest`). This lab worktree may diverge; do not treat lab HEAD as the published tag.

## What's in 0.8.0

- ACME bootstrap from zero: starts without a certificate on disk, answers HTTP-01 on the cleartext listener, installs the issued certificate without a restart
- TLS listeners fail closed while certificate material is pending
- Route response headers on the wire (`[[route.response_headers]]`, e.g. HSTS)
- Multi-listen: every `[[server]].listen` address is bound and served
- Full-page cache with direct purge
- Security: zstd RUSTSEC-2026-0391..0406 resolved; Docker Scout reports **0 critical / 0 high / 0 medium** on both images and both architectures

Full notes: [v0.8.0 release](https://github.com/ExyonQ/ExyonQ/releases/tag/v0.8.0).

## Get it

```bash
docker pull ghcr.io/exyonq/exyonq:0.8.0
docker pull ghcr.io/exyonq/exyonq-wordpress:0.8.0
# Docker Hub mirror:
docker pull exyonq/exyonq:0.8.0
docker pull exyonq/exyonq-wordpress:0.8.0
```

Release assets on GitHub: `exyonq-v0.8.0-linux-{amd64,arm64}` binaries + `SHA256SUMS`.

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
