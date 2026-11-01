# ExyonQ 0.9.0 and current WordPress, SQLite, one container.
# The HTTP server is the static binary from the official image. PHP-FPM 8.5.12+
# runs on the official php-fpm Debian base (glibc bookworm — the same libc the
# ExyonQ release binary links against). Build the official image first:
#   docker build -f packaging/docker/Dockerfile -t exyonq:0.9.0 .
#   docker build -f packaging/docker/wordpress/Dockerfile -t exyonq-wordpress:0.9.0 .

ARG EXYONQ_IMAGE=exyonq:0.9.0
ARG EXYONQ_VERSION=0.9.0
ARG EXYONQ_GIT_REVISION=unknown

FROM ${EXYONQ_IMAGE} AS exyonqbin

# WordPress payload is fetched in a builder stage so the runtime image carries
# no downloader (curl/unzip).
FROM debian:bookworm-slim AS fetcher
RUN apt-get update && apt-get install -y --no-install-recommends curl unzip ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://wordpress.org/latest.tar.gz | tar -xz -C /tmp \
    && curl -fsSL -o /tmp/sqlite.zip https://downloads.wordpress.org/plugin/sqlite-database-integration.latest-stable.zip \
    && mkdir -p /staging/wp-content/plugins /staging/wp-content/database \
    && cp -a /tmp/wordpress/. /staging/ \
    && unzip -q /tmp/sqlite.zip -d /staging/wp-content/plugins \
    && cp /staging/wp-content/plugins/sqlite-database-integration/db.copy /staging/wp-content/db.php
COPY integrations/wordpress/exyonq-cache/ /staging/wp-content/mu-plugins/exyonq-cache/
COPY packaging/docker/wordpress/mu-plugins/exyonq-cache.php /staging/wp-content/mu-plugins/exyonq-cache.php
COPY packaging/docker/wordpress/wordpress.htaccess /staging/.htaccess
RUN rm -rf /staging/wp-content/mu-plugins/exyonq-cache/tests \
    && find /staging -name .htaccess ! -path /staging/.htaccess -delete

# php:8.5.12-fpm-bookworm ships PHP-FPM 8.5.12 with most WordPress extensions
# already compiled in (curl, mbstring, iconv, pdo, pdo_sqlite, xml, phar, ctype,
# dom, fileinfo, sodium, opcache, openssl). Only intl/exif/gd/zip are built here;
# build deps are purged, keeping only the runtime libraries.
FROM php:8.5.12-fpm-bookworm
ARG EXYONQ_VERSION
ARG EXYONQ_GIT_REVISION
RUN set -eux; \
    php -v | grep -qE '^PHP 8\.5\.(1[2-9]|[2-9][0-9])' \
        || { echo 'ERROR: PHP >= 8.5.12 required in this image' >&2; exit 1; }; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        libicu-dev libzip-dev libpng-dev libjpeg62-turbo-dev libfreetype6-dev \
        libicu72 libzip4 libpng16-16 libjpeg62-turbo libfreetype6; \
    docker-php-ext-configure gd --with-freetype --with-jpeg; \
    docker-php-ext-install -j"$(nproc)" intl exif gd zip; \
    apt-get purge -y --auto-remove -o APT::AutoRemove::RecommendsImportant=false \
        libicu-dev libzip-dev libpng-dev libjpeg62-turbo-dev libfreetype6-dev; \
    rm -rf /var/lib/apt/lists/*; \
    rm -f /usr/local/etc/php-fpm.d/docker.conf /usr/local/etc/php-fpm.d/zz-docker.conf; \
    sed -i 's|^;\?listen = .*|listen = /run/php/php-fpm.sock|' /usr/local/etc/php-fpm.d/www.conf; \
    grep -q '^listen = ' /usr/local/etc/php-fpm.d/www.conf \
        || printf 'listen = /run/php/php-fpm.sock\n' >> /usr/local/etc/php-fpm.d/www.conf; \
    sed -i 's|^user = .*|user = www-data|' /usr/local/etc/php-fpm.d/www.conf; \
    sed -i 's|^group = .*|group = www-data|' /usr/local/etc/php-fpm.d/www.conf; \
    printf '\nlisten.owner = www-data\nlisten.group = www-data\nlisten.mode = 0660\nclear_env = no\n' >> /usr/local/etc/php-fpm.d/www.conf; \
    printf 'expose_php = Off\n' > /usr/local/etc/php/conf.d/exyonq.ini; \
    mkdir -p /run/php
COPY --from=exyonqbin /usr/local/bin/exyonq /usr/local/bin/exyonq
COPY packaging/release/conf/wordpress.toml /etc/exyonq/wordpress.toml
COPY packaging/docker/wordpress/entrypoint.sh /usr/local/bin/exyonq-wordpress-entrypoint
COPY --from=fetcher /staging /var/www/html
RUN chmod 0755 /usr/local/bin/exyonq /usr/local/bin/exyonq-wordpress-entrypoint \
    && chown -R www-data:www-data /var/www/html /run/php
LABEL org.opencontainers.image.title="ExyonQ WordPress" \
      org.opencontainers.image.version="${EXYONQ_VERSION}" \
      org.opencontainers.image.revision="${EXYONQ_GIT_REVISION}" \
      org.opencontainers.image.source="https://github.com/ExyonQ/ExyonQ" \
      org.opencontainers.image.licenses="Apache-2.0"
ENV EXYONQ_CONFIG=/etc/exyonq/wordpress.toml
EXPOSE 8080
ENTRYPOINT ["/bin/sh", "/usr/local/bin/exyonq-wordpress-entrypoint"]
