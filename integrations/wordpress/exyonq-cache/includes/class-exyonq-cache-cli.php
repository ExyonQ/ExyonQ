<?php
/**
 * WP-CLI commands for ExyonQ Cache.
 *
 * @package ExyonQ_Cache
 */

declare(strict_types=1);

if ( ! defined( 'ABSPATH' ) && ! defined( 'EXYONQ_CACHE_TEST_BOOTSTRAP' ) ) {
	exit;
}

final class ExyonQ_Cache_CLI {

	public static function register(): void {
		if ( ! class_exists( 'WP_CLI' ) ) {
			return;
		}
		WP_CLI::add_command( 'exyonq-cache', self::class );
	}

	/**
	 * Show plugin / socket diagnostic status (never prints the token).
	 *
	 * ## EXAMPLES
	 *
	 *     wp exyonq-cache status
	 */
	public function status( $args, $assoc_args ): void {
		unset( $args, $assoc_args );
		$plugin = ExyonQ_Cache_Plugin::instance();
		$client = $plugin->client();
		$probe  = $client->status_probe();

		WP_CLI::log( 'plugin_version=' . EXYONQ_CACHE_PLUGIN_VERSION );
		WP_CLI::log( 'multisite=' . ( is_multisite() ? 'yes(deferred)' : 'no' ) );
		WP_CLI::log( 'configured=' . ( ! empty( $probe['configured'] ) ? 'yes' : 'no' ) );
		WP_CLI::log( 'socket_configured=' . ( $client->socket_path() !== '' ? 'yes' : 'no' ) );
		WP_CLI::log( 'site_id_configured=' . ( $client->site_id() > 0 ? 'yes' : 'no' ) );
		WP_CLI::log( 'reachable=' . ( ! empty( $probe['reachable'] ) ? 'yes' : 'no' ) );
		WP_CLI::log( 'auth=' . ( $probe['auth'] ?? 'unknown' ) );
		if ( ! empty( $probe['error'] ) ) {
			WP_CLI::log( 'error=' . $probe['error'] );
		}
		$last = $client->last_result();
		WP_CLI::log( 'last_result=' . ( $last ?? 'none' ) );
		foreach ( $client->counters() as $k => $v ) {
			WP_CLI::log( $k . '=' . $v );
		}
	}

	/**
	 * Purge one public URL.
	 *
	 * ## OPTIONS
	 *
	 * <url>
	 * : Absolute URL or path beginning with /.
	 *
	 * ## EXAMPLES
	 *
	 *     wp exyonq-cache purge-url https://example.com/hello/
	 */
	public function purge_url( $args, $assoc_args ): void {
		unset( $assoc_args );
		$url = isset( $args[0] ) ? (string) $args[0] : '';
		if ( $url === '' ) {
			WP_CLI::error( 'URL required' );
		}
		ExyonQ_Cache_Plugin::instance()->purge_urls( array( $url ) );
		$last = ExyonQ_Cache_Plugin::instance()->client()->last_result();
		WP_CLI::success( 'purge requested; last_result=' . ( $last ?? 'none' ) );
	}

	/**
	 * Purge all L1 entries for the configured site.
	 *
	 * ## EXAMPLES
	 *
	 *     wp exyonq-cache purge-site
	 */
	public function purge_site( $args, $assoc_args ): void {
		unset( $args, $assoc_args );
		ExyonQ_Cache_Plugin::instance()->purge_site_once( 'cli' );
		$last = ExyonQ_Cache_Plugin::instance()->client()->last_result();
		WP_CLI::success( 'purge site requested; last_result=' . ( $last ?? 'none' ) );
	}

	/**
	 * Purge one runtime generation for the configured site.
	 *
	 * ## OPTIONS
	 *
	 * <generation>
	 * : Runtime generation number.
	 *
	 * ## EXAMPLES
	 *
	 *     wp exyonq-cache purge-generation 12
	 */
	public function purge_generation( $args, $assoc_args ): void {
		unset( $assoc_args );
		$gen = isset( $args[0] ) ? (int) $args[0] : -1;
		if ( $gen < 0 ) {
			WP_CLI::error( 'generation required' );
		}
		ExyonQ_Cache_Plugin::instance()->purge_generation_once( $gen );
		$last = ExyonQ_Cache_Plugin::instance()->client()->last_result();
		WP_CLI::success( 'purge generation requested; last_result=' . ( $last ?? 'none' ) );
	}
}
