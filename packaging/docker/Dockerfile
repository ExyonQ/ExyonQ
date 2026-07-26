# syntax=docker/dockerfile:1
# Official ExyonQ production image (clean tree starts at v0.4.0)
# Legal bundle (LICENSE, NOTICE, THIRD_PARTY_NOTICES.md, sbom.cdx.json) must exist
# in the build context — generate via scripts/legal/generate-release-compliance-artifacts.sh

FROM rust:1.93-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY core ./core
COPY config ./config
COPY addon-api ./addon-api
COPY addon-sdk ./addon-sdk
COPY module-api ./module-api
COPY modules ./modules
COPY compat ./compat
COPY cli ./cli
COPY crates ./crates
COPY wasm ./wasm
COPY tests ./tests
COPY xtask ./xtask
RUN cargo build --release -p exyonq -p exyonqctl -p exyonq-compat-cli

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home --shell /usr/sbin/nologin exyonq
COPY --from=builder /app/target/release/exyonq /usr/local/bin/exyonq
COPY --from=builder /app/target/release/exyonqctl /usr/local/bin/exyonqctl
COPY LICENSE NOTICE THIRD_PARTY_NOTICES.md sbom.cdx.json /usr/share/doc/exyonq/
COPY packaging/config/config.toml.example /etc/exyonq/config.toml
ENV EXYONQ_CONFIG=/etc/exyonq/config.toml
ENV EXYONQ_CONTROL_SOCKET=/tmp/exyonq.sock
EXPOSE 8080
USER exyonq
ENTRYPOINT ["exyonq"]
CMD ["serve", "-c", "/etc/exyonq/config.toml"]
