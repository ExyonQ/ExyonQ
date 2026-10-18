# ExyonQ architecture (source tree)

ExyonQ is a modular reverse proxy. This clean publication tree contains the compilable product sources without the internal `docs/` and `benchmarks/` evidence packs.

## Layout

| Path | Role |
|------|------|
| `core/` | Data-plane kernel and HTTP runtime |
| `crates/` | Modules (static, proxy, TLS, HTTP/3, FastCGI, …) and platform |
| `cli/` | `exyonq`, `exyonqctl`, compat CLI |
| `config/` | Config IR / merge / schema / surface |
| `modules/` | Optional modules (metrics, compression, ratelimit, ACME) |
| `wasm/` | WASM host |
| `packaging/` | Packaging contracts and container helpers |
| `tests/` | Integration fixtures and tests |
| `xtask/` | Maintainer automation |
| `fuzz/` | Fuzz targets |

## Defaults

- Allocator: system (optional jemalloc feature)
- HTTP/3: s2n default; quiche optional; quinn-legacy rollback
- Mimalloc: not a product feature

## Release

Published release: **v0.5.0**, commit `e2f0a3827d71f8f6a92c20e4fcfff954432b10c7`. The production container is Debian bookworm slim. `exyonq` and `exyonqctl` are glibc builds, with CA certificates and a default page. v0.4.13 remains available as the previous scratch image with static musl binaries. Current tags, with no `:latest`: `exyonq/exyonq:0.6.0` and `exyonq/exyonq-wordpress:0.6.0` on GHCR and Docker Hub.

## Contact

- contact@exyonq.org
- security@exyonq.org

## License

Apache-2.0 — see `LICENSE` / `NOTICE`.
