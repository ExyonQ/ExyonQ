# ExyonQ 0.7.0 and current WordPress, SQLite, one container.
# The HTTP server is the static binary from the official image. PHP-FPM runs on
# Wolfi (Chainguard) — minimal CVE surface (Docker Sponsored OSS gate:
# Scout 0 critical / 0 high / 0 medium). Build the official image first:
#   docker build -f packaging/docker/Dockerfile -t exyonq:0.7.0 .
#   docker build -f packaging/docker/wordpress/Dockerfile -t exyonq-wordpress:0.7.0 .

ARG EXYONQ_IMAGE=exyonq:0.7.0
ARG EXYONQ_VERSION=0.7.0
ARG EXYONQ_GIT_REVISION=unknown

FROM ${EXYONQ_IMAGE} AS exyonqbin

# WordPress payload is fetched in a builder stage so the runtime image carries
# no downloader (curl/unzip) and no apk write to the final layer besides PHP.
FROM cgr.dev/chainguard/wolfi-base:latest AS fetcher
RUN apk add --no-cache curl unzip
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

FROM cgr.dev/chainguard/wolfi-base:latest
ARG EXYONQ_VERSION
ARG EXYONQ_GIT_REVISION
# PHP-FPM + the modules WordPress and the SQLite integration plugin need.
# session/tokenizer/zlib are already in the php-8.4 base package.
RUN apk add --no-cache \
        ca-certificates \
        php-8.4 php-8.4-fpm \
        php-8.4-curl php-8.4-mbstring php-8.4-iconv php-8.4-pdo php-8.4-pdo_sqlite \
        php-8.4-xml php-8.4-phar php-8.4-ctype php-8.4-dom php-8.4-fileinfo \
        php-8.4-intl php-8.4-exif php-8.4-gd php-8.4-zip php-8.4-sodium php-8.4-opcache \
        php-8.4-openssl \
    && adduser -D -u 33 www-data \
    && sed -i 's|^listen = .*|listen = /run/php/php-fpm.sock|' /etc/php/php-fpm.d/www.conf \
    && sed -i 's|^listen = .*|listen = /run/php/php-fpm.sock|' /etc/php/php-fpm.d/zz-apko.conf \
    && sed -i 's|^user = .*|user = www-data|' /etc/php/php-fpm.d/www.conf \
    && sed -i 's|^group = .*|group = www-data|' /etc/php/php-fpm.d/www.conf \
    && printf '\nlisten.owner = www-data\nlisten.group = www-data\nlisten.mode = 0660\nclear_env = no\n' >> /etc/php/php-fpm.d/www.conf \
    && printf 'expose_php = Off\n' >> /etc/php/php.ini \
    && mkdir -p /run/php
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
