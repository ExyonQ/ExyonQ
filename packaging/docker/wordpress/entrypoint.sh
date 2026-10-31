#!/bin/sh
# Start PHP-FPM, then ExyonQ. Both run as www-data so the Unix socket is usable.
set -eu

# Fixed SQLite path: one predictable file, no randomized .ht.<hex>/ directory
# and no db-path.php indirection. Override with EXYONQ_WP_DB_PATH to move the
# database outside the docroot or onto its own volume.
EXYONQ_WP_DB_PATH="${EXYONQ_WP_DB_PATH:-/var/www/html/wp-content/database/db.sqlite}"
EXYONQ_WP_DB_DIR="$(dirname "$EXYONQ_WP_DB_PATH")"

mkdir -p /run/php /run/exyonq "$EXYONQ_WP_DB_DIR" /var/www/html/wp-content/mu-plugins
chown www-data:www-data /run/php /run/exyonq "$EXYONQ_WP_DB_DIR" /var/www/html/wp-content/mu-plugins
chmod 0750 /run/exyonq

# The SQLite plugin writes 'DENY FROM ALL' here only when the file is missing;
# the htaccess overlay cannot compile that directive and every restart after
# install then 502s the site. The route-level dotfile/extension deny already
# covers the database file, so a placeholder is enough.
printf '# Database directory protected by ExyonQ route rules.\n' > "$EXYONQ_WP_DB_DIR/.htaccess"
chown www-data:www-data "$EXYONQ_WP_DB_DIR/.htaccess"

# One-shot migration from the randomized layout of pre-0.8 images: when the
# fixed file is missing but a .ht.<hex>/.ht.sqlite database exists, move it to
# DB_PATH and drop the db-path.php indirection.
if [ ! -f /var/www/html/wp-config.php ] || ! grep -q "DB_PATH" /var/www/html/wp-config.php; then
  if [ ! -f "$EXYONQ_WP_DB_PATH" ]; then
    for f in /var/www/html/wp-content/database/.ht.*/.ht.sqlite; do
      if [ -f "$f" ]; then
        mv "$f" "$EXYONQ_WP_DB_PATH"
        rm -rf "${f%/.ht.sqlite}" /var/www/html/wp-content/database/db-path.php
        chown www-data:www-data "$EXYONQ_WP_DB_PATH"
        break
      fi
    done
  fi
fi

if [ ! -f /var/www/html/wp-content/mu-plugins/exyonq-cache.php ]; then
  cat > /var/www/html/wp-content/mu-plugins/exyonq-cache.php << 'EOF'
<?php
/**
 * Plugin Name: ExyonQ Cache
 * Description: Invalida la caché de página de ExyonQ al publicar.
 * Version: 0.2.0
 * Author: ExyonQ
 */
require_once __DIR__ . '/exyonq-cache/exyonq-cache.php';
EOF
  chown www-data:www-data /var/www/html/wp-content/mu-plugins/exyonq-cache.php
fi

export EXYONQ_CACHE_PURGE_SOCKET=/run/exyonq/cache-purge.sock
# Token de purge por arranque: compartido entre ExyonQ (env) y el mu-plugin (wp-config define).
export EXYONQ_CACHE_PURGE_TOKEN="$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')"
# site_id = stable_fpc_site_id("wordpress") — debe coincidir con [[route]] name="wordpress"
# en packaging/release/conf/wordpress.toml.
EXYONQ_WP_SITE_ID=18218855368947783640

if [ ! -f /var/www/html/wp-config.php ]; then
  php -r 'echo file_get_contents("https://api.wordpress.org/secret-key/1.1/salt/");' > /tmp/wp-salts.php
  cat > /var/www/html/wp-config.php << 'EOF'
<?php
define( 'DB_NAME', 'wordpress' );
define( 'DB_USER', 'wordpress' );
define( 'DB_PASSWORD', 'wordpress' );
define( 'DB_HOST', 'localhost' );
define( 'DB_CHARSET', 'utf8' );
define( 'DB_COLLATE', '' );
EOF
  cat >> /var/www/html/wp-config.php << EOF
define( 'DB_PATH', '${EXYONQ_WP_DB_PATH}' );
define( 'EXYONQ_CACHE_PURGE_SOCKET', '${EXYONQ_CACHE_PURGE_SOCKET}' );
define( 'EXYONQ_CACHE_PURGE_TOKEN', '${EXYONQ_CACHE_PURGE_TOKEN}' );
define( 'EXYONQ_CACHE_SITE_ID', '${EXYONQ_WP_SITE_ID}' );
EOF
  cat /tmp/wp-salts.php >> /var/www/html/wp-config.php
  cat >> /var/www/html/wp-config.php << 'EOF'
$table_prefix = 'wp_';
define( 'WP_DEBUG', false );
if ( ! defined( 'ABSPATH' ) ) {
	define( 'ABSPATH', __DIR__ . '/' );
}
require_once ABSPATH . 'wp-settings.php';
EOF
  rm -f /tmp/wp-salts.php
  chown www-data:www-data /var/www/html/wp-config.php
  chmod 0640 /var/www/html/wp-config.php
elif ! grep -q "DB_PATH" /var/www/html/wp-config.php; then
  # Pre-0.8 wp-config: pin the SQLite file to the fixed path.
  awk -v line="define( 'DB_PATH', '${EXYONQ_WP_DB_PATH}' );" \
    '{ print } !done && /<\?php/ { print line; done = 1 }' \
    /var/www/html/wp-config.php > /tmp/wp-config.php \
    && cat /tmp/wp-config.php > /var/www/html/wp-config.php \
    && rm -f /tmp/wp-config.php \
    && chown www-data:www-data /var/www/html/wp-config.php
fi

php-fpm --daemonize
# su(1) exists in both util-linux (Debian) and busybox (Wolfi); runuser does not.
exec su -s /bin/sh www-data -c '/usr/local/bin/exyonq serve -c /etc/exyonq/wordpress.toml'
