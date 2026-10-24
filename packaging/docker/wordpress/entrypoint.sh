#!/bin/sh
# Start PHP-FPM, then ExyonQ. Both run as www-data so the Unix socket is usable.
set -eu

mkdir -p /run/php /run/exyonq /var/www/html/wp-content/database /var/www/html/wp-content/mu-plugins
chown www-data:www-data /run/php /run/exyonq /var/www/html/wp-content/database /var/www/html/wp-content/mu-plugins
chmod 0750 /run/exyonq

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
fi

php-fpm --daemonize
# su(1) exists in both util-linux (Debian) and busybox (Wolfi); runuser does not.
exec su -s /bin/sh www-data -c '/usr/local/bin/exyonq serve -c /etc/exyonq/wordpress.toml'
