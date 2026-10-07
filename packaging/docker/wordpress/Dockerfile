# ExyonQ 0.4.6 and current WordPress, SQLite, one container.
# The HTTP server is the static binary from the official image. PHP-FPM is
# the Debian PHP image. Build the official image first:
#   docker build -f packaging/docker/Dockerfile -t exyonq:0.4.6 .
#   docker build -f packaging/docker/wordpress/Dockerfile -t exyonq-wordpress:0.4.6 .

ARG EXYONQ_IMAGE=exyonq:0.4.6
ARG EXYONQ_VERSION=0.4.6
ARG EXYONQ_GIT_REVISION=unknown

FROM ${EXYONQ_IMAGE} AS exyonqbin

FROM php:8.4-fpm-bookworm
ARG EXYONQ_VERSION=0.4.6
ARG EXYONQ_GIT_REVISION=unknown
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl unzip \
    && rm -rf /var/lib/apt/lists/*
COPY --from=exyonqbin /exyonq /usr/local/bin/exyonq
COPY packaging/release/conf/wordpress.toml /etc/exyonq/wordpress.toml
COPY packaging/docker/wordpress/entrypoint.sh /usr/local/bin/exyonq-wordpress-entrypoint
RUN chmod 0755 /usr/local/bin/exyonq /usr/local/bin/exyonq-wordpress-entrypoint \
    && sed -i 's|^listen = .*|listen = /run/php/php-fpm.sock|' /usr/local/etc/php-fpm.d/www.conf \
    && if [ -f /usr/local/etc/php-fpm.d/docker.conf ]; then sed -i 's|^listen = .*|listen = /run/php/php-fpm.sock|' /usr/local/etc/php-fpm.d/docker.conf; fi \
    && printf '\nlisten.owner = www-data\nlisten.group = www-data\nlisten.mode = 0660\nclear_env = no\n' >> /usr/local/etc/php-fpm.d/www.conf \
    && printf 'expose_php = Off\n' > /usr/local/etc/php/conf.d/zz-exyonq.ini \
    && curl -fsSL https://wordpress.org/latest.tar.gz | tar -xz -C /tmp \
    && rm -rf /var/www/html \
    && mv /tmp/wordpress /var/www/html \
    && curl -fsSL -o /tmp/sqlite.zip https://downloads.wordpress.org/plugin/sqlite-database-integration.latest-stable.zip \
    && unzip -q /tmp/sqlite.zip -d /var/www/html/wp-content/plugins \
    && cp /var/www/html/wp-content/plugins/sqlite-database-integration/db.copy /var/www/html/wp-content/db.php \
    && mkdir -p /var/www/html/wp-content/database /run/php
COPY integrations/wordpress/exyonq-cache/ /var/www/html/wp-content/mu-plugins/exyonq-cache/
COPY packaging/docker/wordpress/mu-plugins/exyonq-cache.php /var/www/html/wp-content/mu-plugins/exyonq-cache.php
RUN rm -rf /var/www/html/wp-content/mu-plugins/exyonq-cache/tests \
    && chown -R www-data:www-data /var/www/html /run/php
LABEL org.opencontainers.image.title="ExyonQ WordPress" \
      org.opencontainers.image.version="${EXYONQ_VERSION}" \
      org.opencontainers.image.revision="${EXYONQ_GIT_REVISION}" \
      org.opencontainers.image.source="https://github.com/ExyonQ/ExyonQ" \
      org.opencontainers.image.licenses="Apache-2.0"
ENV EXYONQ_CONFIG=/etc/exyonq/wordpress.toml
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/exyonq-wordpress-entrypoint"]
