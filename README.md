# ExyonQ

ExyonQ is a modular HTTP server and reverse proxy written in Rust: static files, FastCGI/PHP-FPM, TLS, HTTP/2 and HTTP/3, full page cache, WAF, and a balancer with nginx/Apache-compatible configuration migration.

This repository is a **clean source publication** of the product tree (compile / maintain). Internal benchmark suites, methodology docs, and evidence packs are intentionally not included.

Lab integration tree. Public product release: **v0.8.0** on [exyonq-github](https://github.com/ExyonQ/ExyonQ/releases/tag/v0.8.0) (`ghcr.io/exyonq/exyonq:0.8.0`, `ghcr.io/exyonq/exyonq-wordpress:0.8.0`, `exyonq/exyonq:0.8.0` on Docker Hub — no `:latest`). This lab worktree may diverge; do not treat lab HEAD as the published tag.

## What's in 0.8.0

- PROXY protocol v1/v2 inbound; `set_real_ip_from` X-Forwarded-For trust model
- Sticky sessions (cookie/source/header) + `leastconn`, consistent-hash, P2C upstream selection
- Upstream `backup` / `down` / `max_fails` / `max_conns`, hot `BalancerMember` admin via `exyonqctl`
- Active health checks with expect rules, passive circuit breaking, periodic upstream DNS re-resolution
- `ProxyPassReverse` / `proxy_redirect` response rewriting (`Location`/`Content-Location`/`URI`)
- Encoding cache (gzip/brotli) with quotas on the static dataplane
- WordPress image with end-to-end cache purge on publish (Unix socket protocol)
- Supply chain: cargo-auditable embedded SBOM, Rust 1.99.1, `cargo audit` 0 vulnerabilities
- Docker bases: distroless `cc-debian12:nonroot` (server, ~23 MB) and Wolfi + PHP 8.4-FPM (WordPress) — Docker Scout **0 critical / 0 high / 0 medium** on both images and both architectures

Full notes: [`docs/release/v0.7.0.md`](docs/release/v0.7.0.md) — security audit: `docs/security/audit-v0.7.0.md` (laboratorio).

## Get it

```bash
docker pull ghcr.io/exyonq/exyonq:0.8.0
docker pull ghcr.io/exyonq/exyonq-wordpress:0.8.0
# Docker Hub mirror:
docker pull exyonq/exyonq:0.8.0
docker pull exyonq/exyonq-wordpress:0.8.0
```

Release assets on GitHub: `exyonq-0.7.0-linux-{amd64,arm64}.tar.gz` (versioned layout), `exyonq-linux-{amd64,arm64}.tar.gz` (flat), `SHA256SUMS.txt`, `sbom.cdx.json`, `THIRD_PARTY_NOTICES.md`. Linux binaries are built with `cargo auditable` — verify the embedded SBOM with `cargo audit bin`.

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
