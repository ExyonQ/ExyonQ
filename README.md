# ExyonQ

ExyonQ is a modular reverse proxy written in Rust.

This repository is a **clean source publication** of the product tree (compile / maintain). Internal benchmark suites, methodology docs, and evidence packs are intentionally not included.

Current product version: **v0.4.11**. Release: https://github.com/ExyonQ/ExyonQ/releases/tag/v0.4.11. No `:latest` tag.

## Container images

Public. No GitHub login.

GitHub Container Registry:

```bash
docker pull ghcr.io/exyonq/exyonq:0.4.11
docker pull ghcr.io/exyonq/exyonq-wordpress:0.4.11
```

Docker Hub:

```bash
docker pull exyonq/exyonq:0.4.11
docker pull exyonq/exyonq-wordpress:0.4.11
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
cargo run -p exyonq-compat -- migrate --from nginx --input nginx.conf --out exyonq.toml --report report.md
cargo run -p exyonq-compat -- migrate --from apache --input httpd.conf --out exyonq.toml --report report.md
```

## License

Apache License 2.0 — see `LICENSE` and `NOTICE`.
