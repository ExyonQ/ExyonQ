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

Current product version: **v0.4.7**. The production container is a scratch image: the `exyonq` and `exyonqctl` binaries are static musl, and the image carries CA certificates plus the example config. It does not ship a Debian userland.

## Contact

- contact@exyonq.org
- security@exyonq.org

## License

Apache-2.0 — see `LICENSE` / `NOTICE`.
