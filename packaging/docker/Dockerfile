# Official ExyonQ production image (clean tree starts at v0.4.0)
# Note: no `# syntax=docker/dockerfile:*` pin — avoids an extra registry frontend
# resolve; standard BuildKit parses this Dockerfile.
# Legal bundle (LICENSE, NOTICE, THIRD_PARTY_NOTICES.md, sbom.cdx.json) must exist
# in the build context — generate via scripts/legal/generate-release-compliance-artifacts.sh
#
# Bases are Wolfi/Chainguard multi-arch OCI index digests (linux/amd64 +
# linux/arm64). Owner rule: every image ExyonQ builds uses Wolfi; no Debian,
# Ubuntu, Alpine or Debian-distroless base.
# Revision is injected at build time from the host (do not copy .git; do not hard-code a freeze SHA).
#
# Contract (P14V042):
#   DEVELOPMENT_BUILD_WITHOUT_REVISION = ALLOWED_BUT_MARKED_UNKNOWN
#   OFFICIAL_RELEASE_BUILD_WITHOUT_REVISION = FAIL_CLOSED
#   OFFICIAL_RELEASE_BUILD_WITHOUT_TAG = FAIL_CLOSED
# Pass EXYONQ_GIT_REVISION=<40-hex>, EXYONQ_SOURCE_TAG=v<version> and
# EXYONQ_OFFICIAL_RELEASE=1 for official images. The tag is injected because the
# build context excludes .git (git describe cannot run inside the builder).

ARG EXYONQ_VERSION=0.9.2
ARG EXYONQ_GIT_REVISION=unknown
ARG EXYONQ_OFFICIAL_RELEASE=0
ARG EXYONQ_SOURCE_TAG=""

# Wolfi builder with the workspace-pinned rustc 1.99.1. The -dev variant carries
# apk so the build can add perl/git when a crate's build script needs them.
FROM cgr.dev/chainguard/rust:latest-dev@sha256:fc2f649f494a11960920b7bd776770b89a22251e3dc6f3ffc47fa9d5b5f9a2ef AS builder
# Chainguard images default to nonroot; the build stage needs root for apk and
# for installing cargo-auditable under /usr/local. The runtime stays nonroot.
USER root
ARG EXYONQ_VERSION=0.9.2
ARG EXYONQ_GIT_REVISION=unknown
ARG EXYONQ_OFFICIAL_RELEASE=0
ARG EXYONQ_SOURCE_TAG=""
# Map OCI label/build args into the cargo build-script env (same SHA for amd64/arm64).
ENV EXYONQ_SOURCE_REVISION=${EXYONQ_GIT_REVISION}
ENV EXYONQ_OFFICIAL_RELEASE=${EXYONQ_OFFICIAL_RELEASE}
ENV EXYONQ_SOURCE_TAG=${EXYONQ_SOURCE_TAG}
RUN apk update && apk add --no-cache perl git && rm -rf /var/cache/apk/*
WORKDIR /app
# Fail closed before compile when building an official release image without a real SHA/tag.
RUN set -eu; \
    rev="${EXYONQ_SOURCE_REVISION}"; \
    off="${EXYONQ_OFFICIAL_RELEASE}"; \
    ver="${EXYONQ_VERSION}"; \
    tag="${EXYONQ_SOURCE_TAG}"; \
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
        [ "${tag}" = "v${ver}" ] || { \
          echo "ERROR: official OCI build requires EXYONQ_SOURCE_TAG=v${ver} (got '${tag}')" >&2; \
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
RUN cargo install cargo-auditable --locked --root /usr/local \
    && cargo auditable build --release --locked -p exyonq -p exyonqctl -p exyonq-compat-cli \
    && mkdir -p /var/www/exyonq \
    && printf '%s\n' '<!doctype html><title>ExyonQ</title><p>ExyonQ</p>' > /var/www/exyonq/index.html

# Wolfi runtime: no shell, no package manager — minimal CVE surface.
# Provides glibc, ca-certificates and the nonroot user (uid 65532).
FROM cgr.dev/chainguard/glibc-dynamic:latest@sha256:671a621016fb33812136dea6182b1db957996d3c8c618187aaf86abd9b14af53 AS runtime
ARG EXYONQ_VERSION
ARG EXYONQ_GIT_REVISION
COPY --from=builder /app/target/release/exyonq /usr/local/bin/exyonq
COPY --from=builder /app/target/release/exyonqctl /usr/local/bin/exyonqctl
COPY --from=builder /app/target/release/exyonq-compat /usr/local/bin/exyonq-compat
COPY LICENSE NOTICE THIRD_PARTY_NOTICES.md sbom.cdx.json /usr/share/doc/exyonq/
COPY packaging/docker/default.toml /etc/exyonq/config.toml
COPY --from=builder --chown=65532:65532 /var/www/exyonq /var/www/exyonq
ENV EXYONQ_CONFIG=/etc/exyonq/config.toml
ENV EXYONQ_CONTROL_SOCKET=/tmp/exyonq.sock
LABEL org.opencontainers.image.title="ExyonQ" \
      org.opencontainers.image.version="${EXYONQ_VERSION}" \
      org.opencontainers.image.revision="${EXYONQ_GIT_REVISION}" \
      org.opencontainers.image.source="https://github.com/ExyonQ/ExyonQ" \
      org.opencontainers.image.licenses="Apache-2.0"
EXPOSE 8080
USER nonroot
ENTRYPOINT ["exyonq"]
CMD ["serve", "-c", "/etc/exyonq/config.toml"]
