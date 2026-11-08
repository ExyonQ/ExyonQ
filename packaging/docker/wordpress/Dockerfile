# ExyonQ and current WordPress, SQLite, one container.
# The HTTP server is the static binary from the official image. PHP-FPM 8.5.12+
# runs on Wolfi (Chainguard wolfi-base + php-8.5 packages). Owner rule: every
# image ExyonQ builds uses Wolfi; no Debian, Ubuntu, Alpine or Debian-distroless
# base. Build the official image first:
#   docker build -f packaging/docker/Dockerfile -t exyonq:0.9.2 .
#   docker build -f packaging/docker/wordpress/Dockerfile -t exyonq-wordpress:0.9.2 .

ARG EXYONQ_IMAGE=exyonq:0.9.2
ARG EXYONQ_VERSION=0.9.2
ARG EXYONQ_GIT_REVISION=unknown

FROM ${EXYONQ_IMAGE} AS exyonqbin

# WordPress payload is fetched in a builder stage so the runtime image carries
# no downloader (curl/unzip).
FROM cgr.dev/chainguard/wolfi-base:latest@sha256:771146c2dfc880902a185f060c3db5571473c46b267edd98c3119c83068dfb61 AS fetcher
RUN apk update && apk add --no-cache curl unzip ca-certificates \
    && rm -rf /var/cache/apk/*
RUN curl -fsSL https://wordpress.org/latest.tar.gz | tar -xz -C /tmp \
    && curl -fsSL -o /tmp/sqlite.zip https://downloads.wordpress.org/plugin/sqlite-database-integration.latest-stable.zip \
    && mkdir -p /staging/wp-content/plugins /staging/wp-content/database \
    && cp -a /tmp/wordpress/. /staging/ \
    && unzip -q /tmp/sqlite.zip -d /staging/wp-content/plugins \
    && cp /staging/wp-content/plugins/sqlite-database-integration/db.copy /staging/wp-content/db.php
# The cache invalidation client is a normal plugin, not a must-use plugin.
COPY integrations/wordpress/exyonq-cache/ /staging/wp-content/plugins/exyonq-cache/
COPY packaging/docker/wordpress/wordpress.htaccess /staging/.htaccess
RUN rm -rf /staging/wp-content/plugins/exyonq-cache/tests \
    && find /staging -name .htaccess ! -path /staging/.htaccess -delete

# Wolfi php-8.5 ships PHP-FPM 8.5.12. Extensions are Wolfi packages (intl, gd,
# exif, zip, mysqli, pdo_mysql, pdo_sqlite, sodium, iconv, ctype, phar); opcache
# is built into php-8.5. mysqlnd is installed explicitly because the Wolfi
# mysqli/pdo_mysql metadata does not pull it and both fail to load without it.
FROM cgr.dev/chainguard/wolfi-base:latest@sha256:771146c2dfc880902a185f060c3db5571473c46b267edd98c3119c83068dfb61
ARG EXYONQ_VERSION
ARG EXYONQ_GIT_REVISION
RUN set -eux; \
    apk update; \
    apk add --no-cache \
        php-8.5 php-8.5-fpm php-8.5-pdo php-8.5-mysqlnd php-8.5-mysqli \
        php-8.5-pdo_mysql php-8.5-pdo_sqlite php-8.5-intl php-8.5-gd \
        php-8.5-exif php-8.5-zip php-8.5-curl php-8.5-mbstring php-8.5-xml \
        php-8.5-dom php-8.5-fileinfo php-8.5-simplexml php-8.5-sodium \
        php-8.5-iconv php-8.5-ctype php-8.5-phar \
        busybox util-linux ca-certificates; \
    rm -rf /var/cache/apk/*; \
    addgroup -S -g 82 www-data; \
    adduser -S -D -H -u 82 -G www-data www-data; \
    rm -f /etc/php/php-fpm.d/zz-apko.conf; \
    mkdir -p /run/php /run/exyonq /etc/exyonq; \
    printf 'expose_php = Off\nupload_max_filesize = 64M\npost_max_size = 64M\nopenssl.cafile = /etc/ssl/certs/ca-certificates.crt\ncurl.cainfo = /etc/ssl/certs/ca-certificates.crt\n' > /etc/php/conf.d/exyonq.ini; \
    php -v | grep -qE '^PHP 8\.5\.(1[2-9]|[2-9][0-9])' \
        || { echo 'ERROR: PHP >= 8.5.12 required in this image' >&2; exit 1; }; \
    php -m | grep -qx mysqlnd; \
    php -m | grep -qx mysqli; \
    php -m | grep -qx pdo_mysql
COPY packaging/docker/wordpress/php-fpm-www.conf /etc/php/php-fpm.d/www.conf
COPY --from=exyonqbin /usr/local/bin/exyonq /usr/local/bin/exyonq
COPY packaging/release/conf/wordpress.toml /etc/exyonq/wordpress.toml
COPY packaging/docker/wordpress/entrypoint.sh /usr/local/bin/exyonq-wordpress-entrypoint
COPY --from=fetcher /staging /var/www/html
RUN chmod 0755 /usr/local/bin/exyonq /usr/local/bin/exyonq-wordpress-entrypoint \
    && chown -R www-data:www-data /var/www/html /run/php /run/exyonq
LABEL org.opencontainers.image.title="ExyonQ WordPress" \
      org.opencontainers.image.version="${EXYONQ_VERSION}" \
      org.opencontainers.image.revision="${EXYONQ_GIT_REVISION}" \
      org.opencontainers.image.source="https://github.com/ExyonQ/ExyonQ" \
      org.opencontainers.image.licenses="Apache-2.0"
ENV EXYONQ_CONFIG=/etc/exyonq/wordpress.toml
EXPOSE 8080
# Both PHP-FPM and ExyonQ run as www-data; the image needs no root at runtime.
USER www-data
ENTRYPOINT ["/bin/sh", "/usr/local/bin/exyonq-wordpress-entrypoint"]
