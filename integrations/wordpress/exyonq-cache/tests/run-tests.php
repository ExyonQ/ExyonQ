<?php
/**
 * Standalone test runner (no WordPress / PHPUnit required).
 *
 * Usage (Docker):
 *   docker run --rm -v "$PWD:/work" -w /work/integrations/wordpress/exyonq-cache \
 *     php:8.2-cli php tests/run-tests.php
 */

declare(strict_types=1);

define( 'EXYONQ_CACHE_TEST_BOOTSTRAP', true );

require_once dirname( __DIR__ ) . '/includes/class-exyonq-cache-client.php';
require_once dirname( __DIR__ ) . '/includes/class-exyonq-cache-plugin.php';
require_once dirname( __DIR__ ) . '/includes/class-exyonq-cache-hooks.php';

$failures = 0;

function assert_true( bool $cond, string $msg ): void {
	global $failures;
	if ( ! $cond ) {
		fwrite( STDERR, "FAIL: $msg\n" );
		++$failures;
	} else {
		fwrite( STDOUT, "ok: $msg\n" );
	}
}

/**
 * Start WC3 purge peer socket via a child PHP process (no pcntl required).
 *
 * @return resource|false
 */
function start_purge_peer_server( string $sock_path, string $expected_token, bool $accept_auth = true, int $max_conns = 16 ) {
	if ( file_exists( $sock_path ) ) {
		@unlink( $sock_path );
	}
	$script = __DIR__ . '/purge-peer-server.php';
	$php    = PHP_BINARY !== '' ? PHP_BINARY : 'php';
	$auth   = $accept_auth ? '1' : '0';
	$cmd    = array( $php, $script, $sock_path, $expected_token, $auth, (string) $max_conns );

	$descriptors = array(
		0 => array( 'pipe', 'r' ),
		1 => array( 'pipe', 'w' ),
		2 => array( 'pipe', 'w' ),
	);
	$proc = proc_open( $cmd, $descriptors, $pipes, null, null );
	if ( ! is_resource( $proc ) ) {
		fwrite( STDERR, "cannot start purge peer server\n" );
		exit( 1 );
	}
	fclose( $pipes[0] );
	stream_set_blocking( $pipes[1], false );
	stream_set_blocking( $pipes[2], false );

	$deadline = microtime( true ) + 3.0;
	while ( microtime( true ) < $deadline ) {
		if ( file_exists( $sock_path ) ) {
			usleep( 50000 );
			return $proc;
		}
		usleep( 20000 );
	}
	fwrite( STDERR, "purge peer socket not ready\n" );
	proc_terminate( $proc );
	proc_close( $proc );
	exit( 1 );
}

function stop_purge_peer_server( $proc ): void {
	if ( ! is_resource( $proc ) ) {
		return;
	}
	$status = proc_get_status( $proc );
	if ( ! empty( $status['running'] ) ) {
		proc_terminate( $proc );
	}
	proc_close( $proc );
}

$tmpdir = sys_get_temp_dir() . '/exyonq-cache-test-' . getmypid();
mkdir( $tmpdir, 0700 );
$sock  = $tmpdir . '/purge.sock';
$token = 'test-token-secret';

// --- Unit: configuration missing is non-fatal fail ---
$client = new ExyonQ_Cache_Client();
$r      = $client->purge_site();
assert_true( $r['ok'] === false && ( $r['error'] ?? '' ) === 'configuration_missing', 'missing config fails closed for purge' );

define( 'EXYONQ_CACHE_PURGE_SOCKET', $sock );
define( 'EXYONQ_CACHE_PURGE_TOKEN', $token );
define( 'EXYONQ_CACHE_SITE_ID', 42 );

$client = new ExyonQ_Cache_Client();
assert_true( $client->is_configured(), 'configured when constants set' );
assert_true( $client->socket_path() === $sock, 'socket path accepted' );

// Path traversal rejected at client.
$bad = $client->purge_url( 'http', 'example.test', '/../etc/passwd' );
assert_true( $bad['ok'] === false && ( $bad['error'] ?? '' ) === 'invalid_url', 'path traversal rejected' );

// Socket missing → non-fatal.
$r = $client->purge_url( 'http', 'example.test', '/hello/' );
assert_true( $r['ok'] === false && ( $r['error'] ?? '' ) === 'socket_missing', 'missing socket non-fatal' );

// --- Purge peer socket: valid token ---
$proc = start_purge_peer_server( $sock, $token, true );
$client = new ExyonQ_Cache_Client();
$ok     = $client->purge_url( 'http', 'example.test', '/hello/' );
assert_true( ! empty( $ok['ok'] ), 'valid token purge succeeds' );
$tagged = $client->purge_tag( 'menu' );
assert_true( ! empty( $tagged['ok'] ), 'menu tag purge succeeds' );
$bad_tag = $client->purge_tag( 'post:1' );
assert_true( ( $bad_tag['error'] ?? '' ) === 'invalid_key', 'tag with a colon is rejected' );
assert_true( ( $ok['purged_entries'] ?? 0 ) === 1, 'purged_entries reported' );

if ( ! function_exists( 'home_url' ) ) {
	function home_url( $path = '' ) {
		return 'http://example.test' . $path;
	}
}

$plugin = ExyonQ_Cache_Plugin::instance();
// Cross-call dedupe (simulates multiple hooks for one event).
$plugin->purge_urls( array( 'http://example.test/a/' ) );
$plugin->purge_urls( array( 'http://example.test/a/' ) );
$c = $plugin->client()->counters();
assert_true( ( $c['purge_deduplicated'] ?? 0 ) >= 1, 'duplicate URL deduplicated across hooks' );

// Fanout → site fallback.
$many = array();
for ( $i = 0; $i < 25; $i++ ) {
	$many[] = 'http://example.test/p/' . $i . '/';
}
$before = $plugin->client()->counters()['purge_fallback_site'] ?? 0;
$plugin->purge_urls( $many );
$after = $plugin->client()->counters()['purge_fallback_site'] ?? 0;
assert_true( $after > $before, 'fanout triggers site fallback counter' );

stop_purge_peer_server( $proc );
@unlink( $sock );

// --- Invalid token ---
$proc = start_purge_peer_server( $sock, $token, false );
$client   = new ExyonQ_Cache_Client();
$bad_auth = $client->purge_site();
assert_true( $bad_auth['ok'] === false && ( $bad_auth['error'] ?? '' ) === 'unauthenticated', 'invalid token rejected' );
stop_purge_peer_server( $proc );
@unlink( $sock );

// Token secrecy: ensure last_result never contains token.
$last = $client->last_result() ?? '';
assert_true( ! str_contains( $last, $token ), 'token never in last_result' );

// Malformed JSON response.
$mal_script = $tmpdir . '/malformed-server.php';
file_put_contents(
	$mal_script,
	'<?php
if (file_exists($argv[1])) @unlink($argv[1]);
$s = stream_socket_server("unix://" . $argv[1], $e, $m);
if (!$s) { exit(1); }
chmod($argv[1], 0600);
$c = @stream_socket_accept($s, 5);
if ($c) { fgets($c, 8192); fwrite($c, "{not-json\n"); fclose($c); }
fclose($s);
'
);
$php   = PHP_BINARY !== '' ? PHP_BINARY : 'php';
$mproc = proc_open(
	array( $php, $mal_script, $sock ),
	array( 0 => array( 'pipe', 'r' ), 1 => array( 'pipe', 'w' ), 2 => array( 'pipe', 'w' ) ),
	$mpipes
);
fclose( $mpipes[0] );
$deadline = microtime( true ) + 2.0;
while ( microtime( true ) < $deadline && ! file_exists( $sock ) ) {
	usleep( 10000 );
}
usleep( 50000 );
$client = new ExyonQ_Cache_Client();
$mal    = $client->purge_site();
assert_true( $mal['ok'] === false && ( $mal['error'] ?? '' ) === 'malformed_response', 'malformed response rejected safely' );
stop_purge_peer_server( $mproc );
@unlink( $sock );
@unlink( $mal_script );

// Oversized response (no newline within MAX_RESPONSE_BYTES).
$over_script = $tmpdir . '/oversize-server.php';
file_put_contents(
	$over_script,
	'<?php
if (file_exists($argv[1])) @unlink($argv[1]);
$s = stream_socket_server("unix://" . $argv[1], $e, $m);
if (!$s) { exit(1); }
chmod($argv[1], 0600);
$c = @stream_socket_accept($s, 5);
if ($c) { fgets($c, 8192); fwrite($c, str_repeat("x", 9000)); fclose($c); }
fclose($s);
'
);
$oproc = proc_open(
	array( $php, $over_script, $sock ),
	array( 0 => array( 'pipe', 'r' ), 1 => array( 'pipe', 'w' ), 2 => array( 'pipe', 'w' ) ),
	$opipes
);
fclose( $opipes[0] );
$deadline = microtime( true ) + 2.0;
while ( microtime( true ) < $deadline && ! file_exists( $sock ) ) {
	usleep( 10000 );
}
usleep( 50000 );
$client = new ExyonQ_Cache_Client();
$over   = $client->purge_site();
assert_true( $over['ok'] === false && ( $over['error'] ?? '' ) === 'response_too_large', 'oversized response rejected safely' );
stop_purge_peer_server( $oproc );
@unlink( $sock );
@unlink( $over_script );

// Source hygiene: forbidden PHP constructs (plugin code only; tests may use proc_open).
$root      = dirname( __DIR__ );
$forbidden = array( 'exec(', 'shell_exec(', 'system(', 'passthru(', 'unserialize(', 'eval(' );
foreach ( new RecursiveIteratorIterator( new RecursiveDirectoryIterator( $root ) ) as $file ) {
	if ( ! $file->isFile() || $file->getExtension() !== 'php' ) {
		continue;
	}
	if ( str_contains( $file->getPathname(), '/tests/' ) ) {
		continue;
	}
	$body = file_get_contents( $file->getPathname() );
	foreach ( $forbidden as $fn ) {
		assert_true( ! str_contains( $body, $fn ), 'no ' . $fn . ' in ' . $file->getFilename() );
	}
}

@rmdir( $tmpdir );

if ( $failures > 0 ) {
	fwrite( STDERR, "\n$failures failure(s)\n" );
	exit( 1 );
}
fwrite( STDOUT, "\nAll tests passed.\n" );
exit( 0 );
