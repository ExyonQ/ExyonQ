<?php
/**
 * Plugin orchestrator + per-request deduplication.
 *
 * @package ExyonQ_Cache
 */

declare(strict_types=1);

if ( ! defined( 'ABSPATH' ) && ! defined( 'EXYONQ_CACHE_TEST_BOOTSTRAP' ) ) {
	exit;
}

final class ExyonQ_Cache_Plugin {

	private static ?self $instance = null;

	private ExyonQ_Cache_Client $client;
	private ExyonQ_Cache_Hooks $hooks;

	/** @var array<string,true> */
	private array $dedupe = array();

	public static function instance(): self {
		if ( self::$instance === null ) {
			self::$instance = new self();
		}
		return self::$instance;
	}

	private function __construct() {
		$this->client = new ExyonQ_Cache_Client();
		$this->hooks  = new ExyonQ_Cache_Hooks( $this );
	}

	public function boot(): void {
		$this->hooks->register();
		if ( function_exists( 'add_action' ) && class_exists( 'ExyonQ_Cache_Admin' ) ) {
			ExyonQ_Cache_Admin::register( $this );
		}
	}

	public function client(): ExyonQ_Cache_Client {
		return $this->client;
	}

	/**
	 * Max public URLs purged per WordPress event before site fallback.
	 */
	public function max_urls_per_event(): int {
		if ( defined( 'EXYONQ_CACHE_MAX_URLS_PER_EVENT' ) ) {
			$n = (int) EXYONQ_CACHE_MAX_URLS_PER_EVENT;
			if ( $n > 0 && $n <= 100 ) {
				return $n;
			}
		}
		return 20;
	}

	/**
	 * Purge a list of absolute or path URLs for the configured site.
	 * Fail-open: never throws.
	 *
	 * @param list<string> $urls
	 */
	public function purge_urls( array $urls ): void {
		try {
			if ( ! $this->client->is_configured() ) {
				return;
			}
			$urls = array_values( array_unique( array_filter( array_map( 'strval', $urls ) ) ) );
			if ( count( $urls ) > $this->max_urls_per_event() ) {
				$this->client->note_fallback_site();
				$this->purge_site_once( 'fanout' );
				return;
			}
			$miss = false;
			foreach ( $urls as $url ) {
				if ( $this->purge_one_url( $url ) ) {
					$miss = true;
				}
			}
			if ( $miss ) {
				$this->client->note_fallback_site();
				$this->purge_site_once( 'url_miss' );
			}
		} catch ( Throwable $e ) {
			// Fail-open WordPress.
			unset( $e );
		}
	}

	public function purge_site_once( string $reason = 'site' ): void {
		try {
			if ( ! $this->client->is_configured() ) {
				return;
			}
			$key = 'site:' . $reason;
			if ( isset( $this->dedupe[ $key ] ) ) {
				$this->client->note_deduplicated();
				return;
			}
			$this->dedupe[ $key ] = true;
			$this->client->purge_site();
		} catch ( Throwable $e ) {
			unset( $e );
		}
	}

	public function purge_generation_once( int $generation ): void {
		try {
			if ( ! $this->client->is_configured() ) {
				return;
			}
			$key = 'generation:' . $generation;
			if ( isset( $this->dedupe[ $key ] ) ) {
				$this->client->note_deduplicated();
				return;
			}
			$this->dedupe[ $key ] = true;
			$this->client->purge_generation( $generation );
		} catch ( Throwable $e ) {
			unset( $e );
		}
	}

	/**
	 * @return bool True when ExyonQ had no stored copy, or rejected the URL key.
	 */
	private function purge_one_url( string $url ): bool {
		$parts = $this->parse_public_url( $url );
		if ( $parts === null ) {
			return false;
		}
		$key = implode(
			'|',
			array(
				'url',
				$parts['scheme'],
				$parts['host'],
				$parts['path'],
				$parts['query'],
			)
		);
		if ( isset( $this->dedupe[ $key ] ) ) {
			$this->client->note_deduplicated();
			return false;
		}
		$this->dedupe[ $key ] = true;
		// Cached pages are keyed without a query. A functional query is not a stored object.
		$result = $this->client->purge_url(
			$parts['scheme'],
			$parts['host'],
			$parts['path'],
			''
		);
		$error = isset( $result['error'] ) ? (string) $result['error'] : '';
		return $error === 'not_found' || $error === 'invalid_key';
	}

	/**
	 * @return array{scheme:string,host:string,path:string,query:string}|null
	 */
	public function parse_public_url( string $url ): ?array {
		$url = trim( $url );
		if ( $url === '' ) {
			return null;
		}
		// Path-only → fill from home_url.
		if ( str_starts_with( $url, '/' ) ) {
			$home = function_exists( 'home_url' ) ? home_url( $url ) : $url;
			$url  = $home;
		}
		$p = function_exists( 'wp_parse_url' ) ? wp_parse_url( $url ) : parse_url( $url );
		if ( ! is_array( $p ) || empty( $p['host'] ) ) {
			return null;
		}
		$scheme = isset( $p['scheme'] ) ? strtolower( (string) $p['scheme'] ) : 'http';
		if ( ! in_array( $scheme, array( 'http', 'https' ), true ) ) {
			return null;
		}
		$path = isset( $p['path'] ) ? (string) $p['path'] : '/';
		if ( $path === '' ) {
			$path = '/';
		}
		if ( str_contains( $path, '..' ) ) {
			return null;
		}
		$query = isset( $p['query'] ) ? (string) $p['query'] : '';
		return array(
			'scheme' => $scheme,
			'host'   => strtolower( (string) $p['host'] ),
			'path'   => $path,
			'query'  => $query,
		);
	}
}
