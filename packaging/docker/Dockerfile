# Official ExyonQ production image (clean tree starts at v0.4.0)
# Note: no `# syntax=docker/dockerfile:*` pin — avoids an extra registry frontend
# resolve; standard BuildKit parses this Dockerfile.
# Legal bundle (LICENSE, NOTICE, THIRD_PARTY_NOTICES.md, sbom.cdx.json) must exist
# in the build context — generate via scripts/legal/generate-release-compliance-artifacts.sh
#
# Bases are multi-arch OCI index digests (linux/amd64 + linux/arm64).
# Revision is injected at build time from the host (do not copy .git; do not hard-code a freeze SHA).
#
# Contract (P14V042):
#   DEVELOPMENT_BUILD_WITHOUT_REVISION = ALLOWED_BUT_MARKED_UNKNOWN
#   OFFICIAL_RELEASE_BUILD_WITHOUT_REVISION = FAIL_CLOSED
# Pass EXYONQ_GIT_REVISION=<40-hex> and EXYONQ_OFFICIAL_RELEASE=1 for official images.

ARG EXYONQ_VERSION=0.4.1
ARG EXYONQ_GIT_REVISION=unknown
ARG EXYONQ_OFFICIAL_RELEASE=0

FROM rust:1.93-bookworm@sha256:7c4ae649a84014c467d79319bbf17ce2632ae8b8be123ac2fb2ea5be46823f31 AS builder
ARG EXYONQ_GIT_REVISION=unknown
ARG EXYONQ_OFFICIAL_RELEASE=0
# Map OCI label arg into the cargo build-script env (same SHA for amd64/arm64).
ENV EXYONQ_SOURCE_REVISION=${EXYONQ_GIT_REVISION}
ENV EXYONQ_OFFICIAL_RELEASE=${EXYONQ_OFFICIAL_RELEASE}
WORKDIR /app
# Fail closed before compile when building an official release image without a real SHA.
RUN set -eu; \
    rev="${EXYONQ_SOURCE_REVISION}"; \
    off="${EXYONQ_OFFICIAL_RELEASE}"; \
    case "${off}" in \
      1|true|TRUE|yes|YES) \
        if [ -z "${rev}" ] || [ "${rev}" = "unknown" ]; then \
          echo "ERROR: official OCI build requires EXYONQ_GIT_REVISION=<40-hex> (got '${rev}')" >&2; \
          exit 1; \
        fi; \
        echo "${rev}" | grep -Eq '^[0-9a-f]{40}$' || { \
          echo "ERROR: EXYONQ_GIT_REVISION must be 40 lowercase hex (got '${rev}')" >&2; \
          exit 1; \
        }; \
        ;; \
    esac
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
RUN cargo build --release --locked -p exyonq -p exyonqctl -p exyonq-compat-cli

FROM debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818 AS runtime
ARG EXYONQ_VERSION
ARG EXYONQ_GIT_REVISION
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
LABEL org.opencontainers.image.title="ExyonQ" \
      org.opencontainers.image.version="${EXYONQ_VERSION}" \
      org.opencontainers.image.revision="${EXYONQ_GIT_REVISION}" \
      org.opencontainers.image.source="https://github.com/ExyonQ/ExyonQ" \
      org.opencontainers.image.licenses="Apache-2.0"
EXPOSE 8080
USER exyonq
ENTRYPOINT ["exyonq"]
CMD ["serve", "-c", "/etc/exyonq/config.toml"]
