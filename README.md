# ExyonQ

ExyonQ is a modular HTTP server and reverse proxy written in Rust: static files, FastCGI/PHP-FPM, TLS, HTTP/2 and HTTP/3, full page cache, WAF, and a balancer with nginx/Apache-compatible configuration migration.

This repository is a **clean source publication** of the product tree (compile / maintain). Internal benchmark campaigns and raw evidence packs are intentionally not included; methodology notes and summary dashboards live under `docs/benchmarks/`.

Lab integration tree. Public product release: **v0.9.0** on [exyonq-github](https://github.com/ExyonQ/ExyonQ/releases/tag/v0.9.0) (`ghcr.io/exyonq/exyonq:0.9.0`, `ghcr.io/exyonq/exyonq-wordpress:0.9.0`, `exyonq/exyonq:0.9.0` on Docker Hub — no `:latest`). This lab worktree may diverge; do not treat lab HEAD as the published tag.

## What's in 0.9.0

- Proxy cache keys with vary/bypass semantics, `stale-if-error` serving, and purge by tag/URL/namespace via `exyonqctl`
- `expires`/`cache_headers` runtime application and route `add_header` with inheritance and denylist
- Response body transforms: `gunzip` (bounded streaming decode), `sub_filter` (MIME-scoped), `mirror` (background subrequests), `slice` (range-keyed segment cache)
- OpenLiteSpeed parity: private-cookie cache denial and nonce-safe FPC hard-deny prefixes
- ACME HTTP-01 across processes: filesystem-shared challenge store so redirect + TLS processes validate issuance without DNS API tokens
- WAF surface: `auth_basic`, `zone_ratelimit`, body inspection (`crs`/`multipart`/`xml`), and WAF metrics on `/metrics`
- `exyonq_http_request_duration_seconds` histograms with bounded route cardinality
- NGINX `root`/`alias` semantics: `root_mode` join behavior, exact-file `alias`, directory-redirect flags (`absolute_redirect`, `port_in_redirect`, `server_name_in_redirect` equivalents), and `alias` + `fastcgi_pass` dispatch — see release notes for the remaining deliberate differentials
- WordPress image on PHP **8.5.12** FPM (official bookworm build)
- Supply chain: cargo-auditable embedded SBOM, `cargo audit` 0 vulnerabilities

Full notes: [`docs/release/v0.9.0.md`](docs/release/v0.9.0.md) — security audit: `docs/security/audit-v0.9.0.md`.

## Get it

```bash
docker pull ghcr.io/exyonq/exyonq:0.9.0
docker pull ghcr.io/exyonq/exyonq-wordpress:0.9.0
# Docker Hub mirror:
docker pull exyonq/exyonq:0.9.0
docker pull exyonq/exyonq-wordpress:0.9.0
```

Release assets on GitHub: `exyonq-0.9.0-linux-{amd64,arm64}.tar.gz` (versioned layout), `exyonq-linux-{amd64,arm64}.tar.gz` (flat), `SHA256SUMS.txt`, `sbom.cdx.json`, `THIRD_PARTY_NOTICES.md`. Linux binaries are built with `cargo auditable` — verify the embedded SBOM with `cargo audit bin`.

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
