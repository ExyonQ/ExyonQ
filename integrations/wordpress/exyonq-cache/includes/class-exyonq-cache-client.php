<?php
/**
 * Unix-domain client for the WC3 ExyonQ cache purge socket.
 *
 * @package ExyonQ_Cache
 */

declare(strict_types=1);

if ( ! defined( 'ABSPATH' ) && ! defined( 'EXYONQ_CACHE_TEST_BOOTSTRAP' ) ) {
	exit;
}

/**
 * Speaks WC3 line protocol only. Never shells out to exyonqctl.
 */
final class ExyonQ_Cache_Client {

	public const DEFAULT_CONNECT_TIMEOUT_MS = 200;
	public const DEFAULT_IO_TIMEOUT_MS      = 500;
	public const MAX_REQUEST_BYTES          = 4096;
	public const MAX_RESPONSE_BYTES         = 8192;

	/** @var array<string,int> */
	private array $counters = array(
		'purge_attempt'       => 0,
		'purge_success'       => 0,
		'purge_failure'       => 0,
		'purge_empty'         => 0,
		'purge_fallback_site' => 0,
		'purge_deduplicated'  => 0,
	);

	/** @var string|null In-memory last result (no secrets). */
	private ?string $last_result = null;

	public function is_configured(): bool {
		return $this->socket_path() !== ''
			&& $this->token() !== ''
			&& $this->site_id_decimal() !== '';
	}

	public function socket_path(): string {
		if ( defined( 'EXYONQ_CACHE_PURGE_SOCKET' ) ) {
			return $this->sanitize_socket_path( (string) EXYONQ_CACHE_PURGE_SOCKET );
		}
		return $this->sanitize_socket_path( '/run/exyonq/cache-purge.sock' );
	}

	public function token(): string {
		if ( defined( 'EXYONQ_CACHE_PURGE_TOKEN' ) ) {
			return (string) EXYONQ_CACHE_PURGE_TOKEN;
		}
		return $this->token_from_file();
	}

	/**
	 * ExyonQ site_id for the WC3 line.
	 *
	 * PHP integers are platform-sized (typically signed 64-bit). Values that do not
	 * fit in PHP_INT_MAX must not be used — configure a site_id within PHP int range.
	 */
	public function site_id(): int {
		if ( ! defined( 'EXYONQ_CACHE_SITE_ID' ) ) {
			return 0;
		}
		$raw = EXYONQ_CACHE_SITE_ID;
		if ( is_string( $raw ) ) {
			if ( ! preg_match( '/^[1-9][0-9]{0,18}$/', $raw ) ) {
				return 0;
			}
			// Reject strings that would overflow a signed 64-bit PHP int on this platform.
			if ( strlen( $raw ) > strlen( (string) PHP_INT_MAX )
				|| ( strlen( $raw ) === strlen( (string) PHP_INT_MAX ) && $raw > (string) PHP_INT_MAX ) ) {
				return 0;
			}
		}
		$id = (int) $raw;
		return $id > 0 ? $id : 0;
	}

	/**
	 * Decimal site id for the purge line. Reads `<socket>.sites` when no constant is set.
	 * PHP must not recompute the Rust site id.
	 */
	public function site_id_decimal(): string {
		if ( defined( 'EXYONQ_CACHE_SITE_ID' ) ) {
			$id = $this->site_id();
			return $id > 0 ? (string) $id : '';
		}
		return $this->site_id_from_index();
	}

	/**
	 * Safe fields for the admin screen. Never includes the token.
	 *
	 * @return array{socket:string,socket_present:bool,token_present:bool,site_id:string,configured:bool}
	 */
	public function public_status(): array {
		$socket = $this->socket_path();
		$site   = $this->site_id_decimal();
		return array(
			'socket'         => $socket,
			'socket_present' => $socket !== '' && file_exists( $socket ),
			'token_present'  => $this->token() !== '',
			'site_id'        => $site,
			'configured'     => $this->is_configured(),
		);
	}

	/**
	 * @return array{ok:bool,error?:string,purged_entries?:int,purged_bytes?:int,command?:string}
	 */
	public function purge_url( string $scheme, string $host, string $path, string $query = '' ): array {
		$scheme = strtolower( trim( $scheme ) );
		$host   = strtolower( trim( $host ) );
		$path   = $this->normalize_path( $path );
		$query  = trim( $query );

		if ( ! in_array( $scheme, array( 'http', 'https' ), true ) ) {
			return $this->fail( 'invalid_url' );
		}
		if ( $host === '' || str_contains( $host, '/' ) || str_contains( $host, '?' ) ) {
			return $this->fail( 'invalid_url' );
		}
		if ( $path === '' || ! str_starts_with( $path, '/' ) || str_contains( $path, '..' ) ) {
			return $this->fail( 'invalid_url' );
		}

		$site = $this->site_id_decimal();
		$tok  = $this->token();
		if ( $query === '' ) {
			$line = sprintf( 'purge url %s %s %s %s %s', $site, $scheme, $host, $path, $tok );
		} else {
			$line = sprintf( 'purge url %s %s %s %s %s %s', $site, $scheme, $host, $path, $query, $tok );
		}
		return $this->send_line( $line );
	}

	/**
	 * @return array{ok:bool,error?:string,purged_entries?:int,purged_bytes?:int,command?:string}
	 */
	public function purge_tag( string $tag ): array {
		if ( ! preg_match( '/^[a-z0-9][a-z0-9-]{0,31}$/', $tag ) ) {
			return $this->fail( 'invalid_key' );
		}
		$line = sprintf( 'purge tag %s %s %s', $this->site_id_decimal(), $tag, $this->token() );
		return $this->send_line( $line );
	}

	public function purge_site(): array {
		$line = sprintf( 'purge site %s %s', $this->site_id_decimal(), $this->token() );
		return $this->send_line( $line );
	}

	public function note_fallback_site(): void {
		$this->bump( 'purge_fallback_site' );
	}

	/**
	 * @return array{ok:bool,error?:string,purged_entries?:int,purged_bytes?:int,command?:string}
	 */
	public function purge_generation( int $generation ): array {
		if ( $generation < 0 ) {
			return $this->fail( 'invalid_generation' );
		}
		$line = sprintf( 'purge generation %s %d %s', $this->site_id_decimal(), $generation, $this->token() );
		return $this->send_line( $line );
	}

	public function note_deduplicated(): void {
		$this->bump( 'purge_deduplicated' );
	}

	/**
	 * @return array<string,int>
	 */
	public function counters(): array {
		return $this->counters;
	}

	public function last_result(): ?string {
		return $this->last_result;
	}

	/**
	 * Diagnostic reachability probe (does not log token).
	 *
	 * @return array{configured:bool,reachable:bool,auth:string,error?:string}
	 */
	public function status_probe(): array {
		if ( ! $this->is_configured() ) {
			return array(
				'configured' => false,
				'reachable'  => false,
				'auth'       => 'unknown',
				'error'      => 'configuration_missing',
			);
		}
		// Auth probe without clearing the live generation (purge gen 0 is typically empty).
		$result = $this->purge_generation( 0 );
		if ( ! empty( $result['ok'] ) ) {
			return array(
				'configured' => true,
				'reachable'  => true,
				'auth'       => 'accepted',
			);
		}
		$err = $result['error'] ?? 'unknown';
		$auth = ( $err === 'unauthenticated' ) ? 'rejected' : 'unknown';
		$reachable = ! in_array( $err, array( 'socket_missing', 'connect_failed', 'timeout' ), true );
		return array(
			'configured' => true,
			'reachable'  => $reachable,
			'auth'       => $auth,
			'error'      => $err,
		);
	}

	/**
	 * @return array{ok:bool,error?:string,purged_entries?:int,purged_bytes?:int,command?:string}
	 */
	private function send_line( string $line ): array {
		$this->bump( 'purge_attempt' );

		if ( ! $this->is_configured() ) {
			return $this->fail( 'configuration_missing' );
		}
		if ( strlen( $line ) + 1 > self::MAX_REQUEST_BYTES ) {
			return $this->fail( 'request_too_large' );
		}

		$path = $this->socket_path();
		if ( $path === '' || ! file_exists( $path ) ) {
			return $this->fail( 'socket_missing' );
		}

		$errno  = 0;
		$errstr = '';
		$timeout_s = self::DEFAULT_CONNECT_TIMEOUT_MS / 1000.0;
		$sock = @stream_socket_client(
			'unix://' . $path,
			$errno,
			$errstr,
			$timeout_s,
			STREAM_CLIENT_CONNECT
		);
		if ( ! is_resource( $sock ) ) {
			return $this->fail( 'connect_failed' );
		}

		$io_s = self::DEFAULT_IO_TIMEOUT_MS / 1000.0;
		stream_set_timeout( $sock, (int) floor( $io_s ), (int) ( ( $io_s - floor( $io_s ) ) * 1_000_000 ) );
		stream_set_blocking( $sock, true );

		$payload = $line . "\n";
		$written = @fwrite( $sock, $payload );
		if ( $written === false || $written < strlen( $payload ) ) {
			fclose( $sock );
			return $this->fail( 'write_failed' );
		}

		$response = '';
		while ( strlen( $response ) < self::MAX_RESPONSE_BYTES ) {
			$chunk = @fread( $sock, 1024 );
			if ( $chunk === false || $chunk === '' ) {
				$meta = stream_get_meta_data( $sock );
				if ( ! empty( $meta['timed_out'] ) ) {
					fclose( $sock );
					return $this->fail( 'timeout' );
				}
				break;
			}
			$response .= $chunk;
			if ( str_contains( $response, "\n" ) ) {
				break;
			}
		}
		fclose( $sock );

		if ( strlen( $response ) >= self::MAX_RESPONSE_BYTES && ! str_contains( $response, "\n" ) ) {
			return $this->fail( 'response_too_large' );
		}

		$line_out = trim( strtok( $response, "\n" ) ?: '' );
		if ( $line_out === '' ) {
			return $this->fail( 'empty_response' );
		}

		$decoded = json_decode( $line_out, true );
		if ( ! is_array( $decoded ) ) {
			return $this->fail( 'malformed_response' );
		}

		$ok = ! empty( $decoded['ok'] );
		$out = array(
			'ok'             => $ok,
			'command'        => isset( $decoded['command'] ) ? (string) $decoded['command'] : '',
			'purged_entries' => isset( $decoded['purged_entries'] ) ? (int) $decoded['purged_entries'] : 0,
			'purged_bytes'   => isset( $decoded['purged_bytes'] ) ? (int) $decoded['purged_bytes'] : 0,
		);
		if ( ! $ok ) {
			$error = isset( $decoded['error'] ) ? (string) $decoded['error'] : 'purge_failed';
			if ( ! preg_match( '/^[a-z0-9_]{1,64}$/', $error ) ) {
				$error = 'purge_failed';
			}
			$out['error'] = $error;
			if ( $error === 'not_found' ) {
				$this->bump( 'purge_empty' );
				$this->last_result = 'empty:not_found';
			} else {
				$this->bump( 'purge_failure' );
				$this->last_result = 'fail:' . $error;
			}
			$this->persist_outcome( false, $error, 0, 0, $out['command'] );
			$this->debug_log( 'purge failed: ' . $error );
			return $out;
		}

		$this->bump( 'purge_success' );
		$this->last_result = 'ok:' . ( $out['command'] ?: 'purge' );
		$this->persist_outcome( true, '', $out['purged_entries'], $out['purged_bytes'], $out['command'] );
		return $out;
	}

	/**
	 * @return array{ok:bool,error:string}
	 */
	private function fail( string $error ): array {
		$this->bump( 'purge_failure' );
		$this->last_result = 'fail:' . $error;
		$this->persist_outcome( false, $error, 0, 0, '' );
		$this->debug_log( 'purge failed: ' . $error );
		return array(
			'ok'    => false,
			'error' => $error,
		);
	}

	private function bump( string $key ): void {
		if ( isset( $this->counters[ $key ] ) ) {
			++$this->counters[ $key ];
		}
	}

	private function debug_log( string $message ): void {
		if ( ! defined( 'EXYONQ_CACHE_DEBUG' ) || ! EXYONQ_CACHE_DEBUG ) {
			return;
		}
		// Never include token or raw purge lines.
		if ( function_exists( 'error_log' ) ) {
			error_log( '[exyonq-cache] ' . $message );
		}
	}

	private function persist_outcome( bool $ok, string $error, int $entries, int $bytes, string $command ): void {
		if ( ! function_exists( 'get_option' ) || ! function_exists( 'update_option' ) ) {
			return;
		}
		$stats = get_option( 'exyonq_cache_stats', array() );
		if ( ! is_array( $stats ) ) {
			$stats = array();
		}
		$stats['attempts'] = (int) ( $stats['attempts'] ?? 0 ) + 1;
		if ( $ok ) {
			$stats['success']    = (int) ( $stats['success'] ?? 0 ) + 1;
			$stats['last_ok']    = 1;
			$stats['last_error'] = '';
		} elseif ( $error === 'not_found' ) {
			$stats['empty']      = (int) ( $stats['empty'] ?? 0 ) + 1;
			$stats['last_ok']    = 2;
			$stats['last_error'] = 'not_found';
		} else {
			$stats['failure']    = (int) ( $stats['failure'] ?? 0 ) + 1;
			$stats['last_ok']    = 0;
			$stats['last_error'] = $error;
		}
		$stats['last_at']      = time();
		$stats['last_entries'] = $entries;
		$stats['last_bytes']   = $bytes;
		$stats['last_command'] = $command;
		update_option( 'exyonq_cache_stats', $stats, false );
	}

	private function token_file(): string {
		if ( defined( 'EXYONQ_CACHE_PURGE_TOKEN_FILE' ) ) {
			return $this->sanitize_socket_path( (string) EXYONQ_CACHE_PURGE_TOKEN_FILE );
		}
		return '/run/exyonq/cache-purge.token';
	}

	private function token_from_file(): string {
		$path = $this->token_file();
		if ( $path === '' || ! is_readable( $path ) ) {
			return '';
		}
		$raw = file_get_contents( $path );
		if ( ! is_string( $raw ) ) {
			return '';
		}
		$raw = trim( $raw );
		if ( ! preg_match( '/^[A-Za-z0-9._~+-]+$/', $raw ) ) {
			return '';
		}
		return $raw;
	}

	private function route_name(): string {
		if ( defined( 'EXYONQ_CACHE_ROUTE' ) ) {
			$route = (string) EXYONQ_CACHE_ROUTE;
			if ( preg_match( '/^[A-Za-z0-9._~+-]+$/', $route ) ) {
				return $route;
			}
		}
		return 'wordpress';
	}

	private function site_id_from_index(): string {
		$socket = $this->socket_path();
		if ( $socket === '' ) {
			return '';
		}
		$path = $socket . '.sites';
		if ( ! is_readable( $path ) ) {
			return '';
		}
		$raw = file_get_contents( $path );
		if ( ! is_string( $raw ) ) {
			return '';
		}
		$want = $this->route_name();
		$lines = preg_split( "/\r\n|\n|\r/", $raw );
		if ( ! is_array( $lines ) ) {
			return '';
		}
		foreach ( $lines as $line ) {
			$line = trim( $line );
			if ( $line === '' || ! str_contains( $line, "\t" ) ) {
				continue;
			}
			$pair = explode( "\t", $line, 2 );
			$name = $pair[0];
			$id   = trim( $pair[1] ?? '' );
			if ( $name !== $want ) {
				continue;
			}
			if ( ! preg_match( '/^[1-9][0-9]{0,19}$/', $id ) ) {
				return '';
			}
			return $id;
		}
		return '';
	}

	private function sanitize_socket_path( string $path ): string {
		$path = trim( $path );
		if ( $path === '' || str_contains( $path, "\0" ) ) {
			return '';
		}
		// Reject relative / traversal paths.
		if ( ! str_starts_with( $path, '/' ) || str_contains( $path, '..' ) ) {
			return '';
		}
		return $path;
	}

	private function normalize_path( string $path ): string {
		$path = trim( $path );
		if ( $path === '' ) {
			return '/';
		}
		if ( ! str_starts_with( $path, '/' ) ) {
			$path = '/' . $path;
		}
		return $path;
	}
}
