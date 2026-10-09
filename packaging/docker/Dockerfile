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

ARG EXYONQ_VERSION=0.4.9
ARG EXYONQ_GIT_REVISION=unknown
ARG EXYONQ_OFFICIAL_RELEASE=0

FROM rust:1.97.1-bookworm@sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97 AS builder
ARG EXYONQ_GIT_REVISION=unknown
ARG EXYONQ_OFFICIAL_RELEASE=0
# Map OCI label arg into the cargo build-script env (same SHA for amd64/arm64).
ENV EXYONQ_SOURCE_REVISION=${EXYONQ_GIT_REVISION}
ENV EXYONQ_OFFICIAL_RELEASE=${EXYONQ_OFFICIAL_RELEASE}
ENV EXYONQ_ARTIFACT_VERSION=0.4.9
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
COPY third_party ./third_party
COPY scripts/release/build-production.sh scripts/release/build-production.sh
# Product OCI builds exclude the host-only benchmark orchestrator from their
# container-local Cargo workspace; benchmark tooling is never shipped.
RUN sed -i '\|  "benchmarks/exyonq-bench",|d' Cargo.toml
RUN apt-get update && apt-get install -y --no-install-recommends musl-tools && rm -rf /var/lib/apt/lists/*
RUN bash scripts/release/build-production.sh

# Runtime is scratch. The production binaries are static musl, so the image
# does not ship Debian. Debian bookworm-slim carried Perl, pcre2 and hundreds
# of copyleft packages that are not used by the proxy.
FROM debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818 AS certs
RUN set -eu; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates; \
    rm -rf /var/lib/apt/lists/*; \
    useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin exyonq; \
    mkdir -p /rootfs/tmp /rootfs/etc/ssl/certs /rootfs/usr/local/bin /rootfs/usr/share/doc/exyonq /rootfs/etc/exyonq /rootfs/var/www/exyonq; \
    chmod 1777 /rootfs/tmp; \
    chmod 755 /rootfs/var /rootfs/var/www /rootfs/var/www/exyonq; \
    cp /etc/passwd /etc/group /rootfs/etc/; \
    cp /etc/ssl/certs/ca-certificates.crt /rootfs/etc/ssl/certs/ca-certificates.crt

FROM scratch AS runtime
ARG EXYONQ_VERSION
ARG EXYONQ_GIT_REVISION
COPY --from=certs /rootfs/ /
COPY --from=builder /app/target/release/exyonq /usr/local/bin/exyonq
COPY --from=builder /app/target/release/exyonqctl /usr/local/bin/exyonqctl
COPY packaging/docker/www/index.html /var/www/exyonq/index.html
COPY LICENSE NOTICE THIRD_PARTY_NOTICES.md sbom.cdx.json /usr/share/doc/exyonq/
COPY packaging/docker/default.toml /etc/exyonq/config.toml
ENV EXYONQ_CONFIG=/etc/exyonq/config.toml
ENV EXYONQ_CONTROL_SOCKET=/tmp/exyonq.sock
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
LABEL org.opencontainers.image.title="ExyonQ" \
      org.opencontainers.image.version="${EXYONQ_VERSION}" \
      org.opencontainers.image.revision="${EXYONQ_GIT_REVISION}" \
      org.opencontainers.image.source="https://github.com/ExyonQ/ExyonQ" \
      org.opencontainers.image.licenses="Apache-2.0"
EXPOSE 8080
USER exyonq
ENTRYPOINT ["exyonq"]
CMD ["serve", "-c", "/etc/exyonq/config.toml"]
