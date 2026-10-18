# ExyonQ

**A web server and reverse proxy written in Rust, built for modern Linux deployments and practical migration.**

ExyonQ serves static sites, terminates TLS, proxies application traffic, runs PHP through FastCGI, and supports HTTP/1.1, HTTP/2, and HTTP/3/QUIC. It is available as Linux binaries and multi-architecture container images for `linux/amd64` and `linux/arm64`.

> **Project phase:** Alpha · **Published release:** [v0.4.13](https://github.com/ExyonQ/ExyonQ/releases/tag/v0.4.13) · **Source in this tree:** 0.5.0 (not published) · [Documentation](DOCUMENTATION_INDEX.md) · [Release notes](CHANGELOG.md) · [Security policy](SECURITY.md)

ExyonQ is in alpha. The published release is v0.4.13. The source in this tree is 0.5.0 and is not a release: it has no tag and no container images yet. Later versions may change behavior and configuration.

## Why ExyonQ?

ExyonQ is for teams that want a single, modern edge server for static content, application delivery, and reverse proxying — without treating configuration migration as a rewrite.

- **Memory safety by design.** Rust gives ExyonQ a memory-safety-first foundation that prevents many common memory-corruption errors at the server implementation layer. It is not a claim that any server is immune to every security vulnerability.
- **One server for the common web path.** Static files, reverse proxying, FastCGI/PHP-FPM, TLS, HTTP/2, and HTTP/3/QUIC are part of the same product.
- **Migrate deliberately, not blindly.** The included compatibility CLI imports documented subsets of NGINX and Apache configurations and produces a report. Unsupported or ambiguous directives are rejected rather than silently changing behaviour.
- **Built for current deployments.** Versioned Linux `amd64` and `arm64` binaries, plus matching Docker Hub and GHCR images. Images use explicit version tags; there is intentionally no mutable `:latest` tag.
- **Operationally explicit.** TLS reload/drain and configuration validation are product concerns, not afterthoughts.
- **WordPress path included.** A separate image packages the supported WordPress deployment path, including the bounded `.htaccess` compatibility and page-cache integration shipped by the current release line.

## Quick start

Run the pinned server image on port 8080:

```bash
docker run --rm -p 8080:8080 ghcr.io/exyonq/exyonq:0.4.13
curl http://127.0.0.1:8080/
```

Pull the WordPress image when that is your deployment target:

```bash
docker pull ghcr.io/exyonq/exyonq-wordpress:0.4.13
```

Versioned release artifacts and Linux binaries are available from the [v0.4.13 release](https://github.com/ExyonQ/ExyonQ/releases/tag/v0.4.13).

## What it does today

| Area | Delivered capability |
| --- | --- |
| Web serving | Static file serving with restrictive defaults for sensitive files |
| Compression | gzip, deflate, Brotli, and Zstandard, negotiated from `Accept-Encoding` |
| Protocols | HTTP/1.1, HTTP/2, HTTP/3/QUIC and TLS |
| Application delivery | Reverse proxy and FastCGI/PHP-FPM |
| Routing | Hosts, routes, redirects, upstream selection, and TLS certificate selection by server name |
| Load balancing | Smooth weighted round-robin across upstream endpoints, priority bands, and optional HTTP health checks |
| Firewall | Native request WAF: built-in detectors, custom rules, IP filter, monitor or block, and optional per-IP rate limit or challenge |
| Operations | Configuration CLI, validation, reload/drain, release binaries, and containers |
| Platforms | Linux `amd64` and `arm64` binaries and container images |
| WordPress | Supported WordPress image, front-controller handling, selected `.htaccess` rules, and targeted cache invalidation |
| Migration | Offline NGINX and Apache configuration importers with a generated conversion report |

## Moving from an existing server

ExyonQ is not presented as a 100% drop-in replacement for every configuration of Apache, NGINX, or OpenLiteSpeed. It is an alternative for new deployments and a **phased migration target** for the documented configuration subsets.

Start with a conversion report before changing live traffic:

```bash
cargo run -p exyonq-compat -- migrate \
  --from nginx --input nginx.conf --out exyonq.toml --report nginx-migration-report.md

cargo run -p exyonq-compat -- migrate \
  --from apache --input httpd.conf --out exyonq.toml --report apache-migration-report.md
```

The importer is intentionally fail-closed: a configuration feature that cannot be converted with defined semantics must be reviewed or rewritten explicitly. Read the [compatibility scope](docs/compat/compatibility-scope-v1.md) before planning a production migration.

## When ExyonQ is a good fit

- You want a Rust-based web server and reverse proxy for a new Linux deployment.
- You want to serve static content, proxy applications, or run PHP-FPM without assembling separate edge components for each basic path.
- You are migrating a known, supported subset of an Apache or NGINX deployment and want a conversion report as the first step.
- You need reproducible, pinned container versions across `amd64` and `arm64`.
- You run WordPress and want a supported deployment image rather than an unbounded compatibility promise.

## Security and reliability posture

Rust strengthens the memory-safety boundary, but secure operation still depends on configuration, dependencies, protocols, host hardening, and timely upgrades. ExyonQ therefore does not market memory safety as a blanket security guarantee.

Static routes deny PHP-like and dotfile paths by default while keeping `.well-known` available; an explicit opt-in exists when a deployment genuinely needs a sensitive path. See [SECURITY.md](SECURITY.md) for vulnerability reporting and [CHANGELOG.md](CHANGELOG.md) for the current release scope.

## Build from source

```bash
git clone https://github.com/ExyonQ/ExyonQ.git
cd ExyonQ
cargo build -p exyonq -p exyonqctl --release --locked
```

For configuration, deployment, supported migration profiles, and operational details, start at the [documentation index](DOCUMENTATION_INDEX.md).

## Project status and contribution

ExyonQ is open source under [Apache License 2.0](LICENSE). Contributions and reproducible bug reports are welcome; please read [CONTRIBUTING.md](CONTRIBUTING.md) and the [security policy](SECURITY.md) first.

### Important scope boundary

The project publishes versioned artifacts and documented capabilities. It does **not** claim a universal Apache/NGINX/OpenLiteSpeed replacement, nor a public performance victory based on isolated RPS figures. See the [compatibility scope](docs/compat/compatibility-scope-v1.md) and [release notes](CHANGELOG.md) for the current, evidence-bounded scope.
