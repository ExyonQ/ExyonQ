# ExyonQ

ExyonQ is a modular HTTP server and reverse proxy written in Rust: static files, FastCGI/PHP-FPM, TLS, HTTP/2 and HTTP/3, full page cache, WAF, and a balancer with nginx/Apache-compatible configuration migration.

ExyonQ includes migration tools and aims to progressively expand compatibility with Apache, NGINX, and OpenLiteSpeed configurations. Refer to the documentation for details on the scope of each importer.

This repository is the public product source tree (compile / maintain). Internal benchmark campaigns and raw evidence packs are intentionally not included; methodology notes and summary dashboards live under `docs/benchmarks/`.

Public releases are published on [GitHub Releases](https://github.com/ExyonQ/ExyonQ/releases/latest) and in GHCR as versioned images (`ghcr.io/exyonq/exyonq`, `ghcr.io/exyonq/exyonq-wordpress`); Docker Hub mirrors use the same version tags. No `:latest` tag is published. A local worktree may diverge; do not treat its HEAD as a published tag.

## What's in this release line

- External authentication with fail-closed JWT handling, plus user/zone rate limits
- Client throttling for static and dynamic requests, inbound/outbound bandwidth, and hard/soft connection limits with OLS-compatible close/ban behavior
- Human challenge support: native proof-of-work, monitor mode, reCAPTCHA and hCaptcha, with provider failures denied closed
- WAF with OWASP CRS import and real-corpus differential/dry-run tooling, per-rule metrics, body inspection (`crs`/`multipart`/`xml`), and bounded buffering
- Proxy cache keys with vary/bypass semantics, `stale-if-error`, and purge by tag/URL/namespace via `exyonqctl`
- `expires`/`cache_headers` runtime application and route `add_header` with inheritance and denylist
- Response body transforms: `gunzip`, `sub_filter`, `mirror`, and `slice`
- OpenLiteSpeed parity: private-cookie cache denial and nonce-safe FPC hard-deny prefixes
- ACME HTTP-01 across processes with a filesystem-shared challenge store
- NGINX `root`/`alias` semantics, directory-redirect flags, and `alias` + `fastcgi_pass` dispatch
- WordPress image on PHP **8.5.12** FPM (Wolfi `php-8.5-fpm`)
- Supply chain: cargo-auditable embedded SBOM and `cargo audit`

Full release notes live under [`docs/release/`](docs/release/), with the running history in [`CHANGELOG.md`](CHANGELOG.md).

## Get it

Set `VERSION` to the release you want to run:

```bash
VERSION=<release-version>
docker pull "ghcr.io/exyonq/exyonq:${VERSION}"
docker pull "ghcr.io/exyonq/exyonq-wordpress:${VERSION}"
# Docker Hub mirror:
docker pull "exyonq/exyonq:${VERSION}"
docker pull "exyonq/exyonq-wordpress:${VERSION}"
```

Release assets on GitHub use the same version identity: `exyonq-${VERSION}-linux-{amd64,arm64}.tar.gz` (versioned layout), `exyonq-linux-{amd64,arm64}.tar.gz` (flat), `.deb`/`.rpm` packages, `SHA256SUMS.txt`, `release-manifest.json`, `sbom.cdx.json`, and `THIRD_PARTY_NOTICES.md`. Linux binaries are built with `cargo auditable`; verify the embedded SBOM with `cargo audit bin`.

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
