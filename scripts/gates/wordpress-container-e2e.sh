#!/usr/bin/env bash
# Real WordPress container E2E: install, login, FPC hit, nonce POST, logged-in bypass.
# Test-only. Never print the test password.
set -euo pipefail

IMAGE="${1:-exyonq-wordpress:ng18-arm64}"
NAME="wp-e2e-$$"
PORT=18081
BASE="http://127.0.0.1:${PORT}"
COOKIE="$(mktemp)"
COOKIE2="$(mktemp)"
PW="WP-E2E-$(openssl rand -hex 12)"
OUT="${2:-.exyonq-local/evidence/wordpress-container-e2e}"
mkdir -p "$OUT"

cleanup() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
  rm -f "$COOKIE" "$COOKIE2"
}
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

docker run -d --name "$NAME" -p "${PORT}:8080" "$IMAGE" >/dev/null
for _ in $(seq 1 60); do
  code=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/wp-admin/install.php" || true)
  [ "$code" = "200" ] && break
  sleep 1
done
[ "$code" = "200" ] || fail "install.php not reachable ($code)"

# FPC key for plain permalinks (?p=1), same as the differential.
cat > /tmp/wordpress-e2e.toml <<'EOF'
config_version = 2

[logging]
level = "info"
format = "text"

[[server]]
listen = "0.0.0.0:8080"
routes = ["wp-includes", "wp-content", "wordpress"]

[[route]]
name = "wp-includes"
match = { path = "/wp-includes" }
root = "/var/www/html/wp-includes"

[[route]]
name = "wp-content"
match = { path = "/wp-content" }
root = "/var/www/html/wp-content"

[[route]]
name = "wordpress"
match = { path = "/" }
fastcgi = "php"
htaccess = "overlay"

[[fcgi_pool]]
name = "php"
address = "/run/php/php-fpm.sock"
transport = "unix"
document_root = "/var/www/html"
max_concurrency = 16
idle_timeout_ms = 30000
total_timeout_ms = 120000
checkout_timeout_ms = 5000

[full_page_cache]
enabled = true
default_ttl_seconds = 300
max_ttl_seconds = 3600
max_object_bytes = 4194304
max_total_bytes = 67108864
max_entries = 10000
namespace = 4

[full_page_cache.key]
query = ["p"]
EOF
docker cp /tmp/wordpress-e2e.toml "$NAME":/etc/exyonq/wordpress.toml
docker restart "$NAME" >/dev/null
for _ in $(seq 1 60); do
  code=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/wp-admin/install.php" || true)
  [ "$code" = "200" ] && break
  sleep 1
done
[ "$code" = "200" ] || fail "install.php after restart ($code)"

# Language step, then install.
curl -sS -b "$COOKIE" -c "$COOKIE" -o /dev/null \
  -d 'language=en_US' "$BASE/wp-admin/install.php?step=1"
curl -sS -b "$COOKIE" -c "$COOKIE" -o /tmp/wp-e2e-install.html \
  --data-urlencode 'weblog_title=ExyonQ E2E' \
  --data-urlencode 'user_name=admin' \
  --data-urlencode "admin_password=$PW" \
  --data-urlencode "admin_password2=$PW" \
  --data-urlencode 'admin_email=admin@example.test' \
  --data-urlencode 'blog_public=1' \
  --data-urlencode 'language=en_US' \
  --data-urlencode 'Submit=Install WordPress' \
  "$BASE/wp-admin/install.php?step=2"
grep -q 'Success!' /tmp/wp-e2e-install.html || fail "WordPress install did not succeed"

# wp-config.php must carry all eight non-empty salts. A failed remote fetch used
# to write an empty salts file and leave the site with predictable empty keys.
for key in AUTH_KEY SECURE_AUTH_KEY LOGGED_IN_KEY NONCE_KEY AUTH_SALT SECURE_AUTH_SALT LOGGED_IN_SALT NONCE_SALT; do
  docker exec "$NAME" grep -qE "define\\( '$key', '[^']+' \\);" /var/www/html/wp-config.php \
    || fail "wp-config.php missing non-empty $key"
done

# Login and verify the session reaches wp-admin.
curl -sS -b "$COOKIE" -c "$COOKIE" -o /dev/null "$BASE/wp-login.php"
curl -sS -b "$COOKIE" -c "$COOKIE" -o /dev/null -D /tmp/wp-e2e-login.headers \
  --data-urlencode 'log=admin' \
  --data-urlencode "pwd=$PW" \
  --data-urlencode 'wp-submit=Log In' \
  --data-urlencode "redirect_to=$BASE/wp-admin/" \
  --data-urlencode 'testcookie=1' \
  "$BASE/wp-login.php"
grep -qi 'wordpress_logged_in_' "$COOKIE" || fail "login cookie missing"
code=$(curl -s -b "$COOKIE" -o /dev/null -w '%{http_code}' "$BASE/wp-admin/")
[ "$code" = "200" ] || fail "wp-admin not 200 after login ($code)"

# Test-only nonce fragment.
cat > /tmp/wp-e2e-nonce.php <<'EOF'
<?php
/**
 * Plugin Name: OL02 Test Nonce
 * Description: Test-only nonce fragment; never shipped in an image.
 * Version: 0.0.0
 */
add_action( 'wp_footer', function () {
	$nonce = wp_create_nonce( 'ol02_check' );
	echo '<form method="post" action="' . esc_url( admin_url( 'admin-post.php' ) ) . '">';
	echo '<input type="hidden" name="action" value="ol02_check">';
	echo '<input type="hidden" name="ol02_nonce" value="' . esc_attr( $nonce ) . '">';
	echo '</form>';
} );
add_action( 'admin_post_nopriv_ol02_check', function () {
	$ok = isset( $_POST['ol02_nonce'] ) && wp_verify_nonce( $_POST['ol02_nonce'], 'ol02_check' );
	status_header( $ok ? 200 : 403 );
	header( 'Content-Type: text/plain' );
	echo $ok ? 'OL02 nonce OK' : 'OL02 nonce FAIL';
	exit;
} );
EOF
docker exec "$NAME" mkdir -p /var/www/html/wp-content/plugins/ol02-nonce
docker cp /tmp/wp-e2e-nonce.php "$NAME":/var/www/html/wp-content/plugins/ol02-nonce/ol02-nonce.php
docker exec "$NAME" php -r 'define( "WP_USE_THEMES", false ); require "/var/www/html/wp-load.php"; require_once ABSPATH . "wp-admin/includes/plugin.php"; foreach ( array( "exyonq-cache/exyonq-cache.php", "ol02-nonce/ol02-nonce.php" ) as $plugin ) { $result = activate_plugin( $plugin ); if ( is_wp_error( $result ) ) { fwrite( STDERR, $plugin . ": " . $result->get_error_message() . "\n" ); exit( 1 ); } }'
docker exec "$NAME" test ! -e /var/www/html/wp-content/mu-plugins/exyonq-cache.php \
  || fail "exyonq-cache must not be installed as a mu-plugin"
docker restart "$NAME" >/dev/null
for _ in $(seq 1 60); do
  code=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/wp-admin/install.php" || true)
  [ "$code" = "200" ] && break
  sleep 1
done

# The normal plugin must stay active after a restart, and the purge socket
# must accept its token. status_probe sends a real purge_generation command.
docker exec "$NAME" php -r 'define( "WP_USE_THEMES", false ); require "/var/www/html/wp-load.php"; require_once ABSPATH . "wp-admin/includes/plugin.php"; exit( is_plugin_active( "exyonq-cache/exyonq-cache.php" ) ? 0 : 1 );' \
  || fail "exyonq-cache is not active after restart"
docker exec "$NAME" php -r 'define( "WP_USE_THEMES", false ); require "/var/www/html/wp-load.php"; $probe = ExyonQ_Cache_Plugin::instance()->client()->status_probe(); echo json_encode( $probe ), "\n"; if ( ( $probe["auth"] ?? "" ) !== "accepted" ) { exit( 1 ); }' \
  | tee /tmp/wp-e2e-purge-probe.json \
  || fail "purge socket probe did not authenticate"

# Anonymous cache: miss then hit, same nonce.
rm -f "$COOKIE2"
for i in 1 2; do
  curl -sS -b "$COOKIE2" -c "$COOKIE2" -o "/tmp/wp-e2e-home-$i.html" \
    -D "/tmp/wp-e2e-home-$i.headers" "$BASE/?p=1"
done
grep -qi 'x-cache: miss' /tmp/wp-e2e-home-1.headers || fail "first anonymous request not miss"
grep -qi 'x-cache: hit' /tmp/wp-e2e-home-2.headers || fail "second anonymous request not hit"
NONCE=$(grep -o 'name="ol02_nonce" value="[^"]*"' /tmp/wp-e2e-home-2.html | sed 's/.*value="//; s/"//')
[ -n "$NONCE" ] || fail "cached page has no nonce"
code=$(curl -sS -o /tmp/wp-e2e-nonce-post.txt -w '%{http_code}' \
  --data-urlencode 'action=ol02_check' --data-urlencode "ol02_nonce=$NONCE" \
  "$BASE/wp-admin/admin-post.php")
[ "$code" = "200" ] || fail "nonce POST not 200 ($code)"
grep -q 'OL02 nonce OK' /tmp/wp-e2e-nonce-post.txt || fail "nonce POST rejected"

# Logged-in request must bypass the shared cache.
curl -sS -b "$COOKIE" -o /tmp/wp-e2e-loggedin.html -D /tmp/wp-e2e-loggedin.headers "$BASE/?p=1"
grep -qi 'x-cache: bypass' /tmp/wp-e2e-loggedin.headers || fail "logged-in request not bypass"
grep -qi 'x-cache-reason: cookie_denied' /tmp/wp-e2e-loggedin.headers || fail "bypass reason not cookie_denied"

cat > "$OUT/wordpress-e2e.md" <<EOF
# WordPress container E2E

- Image: $IMAGE
- Install: PASS
- Login + wp-admin 200: PASS
- ExyonQ Cache normal plugin active after restart: PASS
- Mu-plugin absent: PASS
- Purge socket status_probe: auth accepted
- Anonymous FPC miss -> hit: PASS
- Cached nonce accepted by another anonymous client: PASS
- Logged-in X-Cache: bypass / cookie_denied: PASS
EOF
echo "WordPress E2E PASS ($IMAGE)"
