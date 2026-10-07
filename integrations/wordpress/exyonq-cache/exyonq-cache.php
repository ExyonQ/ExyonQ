<?php
/**
 * Plugin Name:       ExyonQ Cache
 * Plugin URI:        https://github.com/ExyonQ/Plugins/tree/main/exyonq-cache
 * Description:       Invalida la caché de página de ExyonQ al publicar. Muestra estadísticas y un botón para purgar.
 * Version:           0.2.0
 * Requires at least: 6.0
 * Requires PHP:      8.0
 * Author:            ExyonQ
 * License:           Apache-2.0
 * Text Domain:       exyonq-cache
 *
 * Configuration (wp-config.php constants only — never wp_options):
 *   define( 'EXYONQ_CACHE_PURGE_SOCKET', '/run/exyonq/cache-purge.sock' );
 *   define( 'EXYONQ_CACHE_PURGE_TOKEN', '…' );
 *   define( 'EXYONQ_CACHE_SITE_ID', 123456789 ); // ExyonQ FPC site_id (u64)
 *
 * Multisite: not supported in v0 (DEFERRED).
 */

declare(strict_types=1);

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

define( 'EXYONQ_CACHE_PLUGIN_VERSION', '0.2.0' );
define( 'EXYONQ_CACHE_PLUGIN_FILE', __FILE__ );
define( 'EXYONQ_CACHE_PLUGIN_DIR', plugin_dir_path( __FILE__ ) );

require_once EXYONQ_CACHE_PLUGIN_DIR . 'includes/class-exyonq-cache-client.php';
require_once EXYONQ_CACHE_PLUGIN_DIR . 'includes/class-exyonq-cache-hooks.php';
require_once EXYONQ_CACHE_PLUGIN_DIR . 'includes/class-exyonq-cache-plugin.php';
require_once EXYONQ_CACHE_PLUGIN_DIR . 'includes/class-exyonq-cache-admin.php';

// WP-CLI: single-site only in v0 (multisite deferred — refuse registration).
if ( defined( 'WP_CLI' ) && WP_CLI && function_exists( 'is_multisite' ) && ! is_multisite() ) {
	require_once EXYONQ_CACHE_PLUGIN_DIR . 'includes/class-exyonq-cache-cli.php';
	ExyonQ_Cache_CLI::register();
}

/**
 * Bootstrap hooks on plugins_loaded (single-site only).
 */
function exyonq_cache_bootstrap(): void {
	if ( is_multisite() ) {
		// Multisite deferred — do not register purge hooks.
		return;
	}
	ExyonQ_Cache_Plugin::instance()->boot();
}
add_action( 'plugins_loaded', 'exyonq_cache_bootstrap' );
