<?php
/**
 * Minimal WC3-compatible purge socket peer for plugin tests.
 *
 * Usage:
 *   php purge-peer-server.php <sock_path> <expected_token> <accept_auth:0|1> [max_conns]
 */

declare(strict_types=1);

if ( $argc < 4 ) {
	fwrite( STDERR, "usage: purge-peer-server.php <sock> <token> <accept_auth> [max_conns]\n" );
	exit( 2 );
}

$sock_path      = $argv[1];
$expected_token = $argv[2];
$accept_auth    = $argv[3] === '1';
$max_conns      = isset( $argv[4] ) ? (int) $argv[4] : 16;

if ( file_exists( $sock_path ) ) {
	@unlink( $sock_path );
}

$server = stream_socket_server( 'unix://' . $sock_path, $errno, $errstr );
if ( ! $server ) {
	fwrite( STDERR, "bind failed: $errstr\n" );
	exit( 1 );
}
chmod( $sock_path, 0600 );
fwrite( STDOUT, "ready\n" );
fflush( STDOUT );

for ( $i = 0; $i < $max_conns; $i++ ) {
	$conn = @stream_socket_accept( $server, 5 );
	if ( ! $conn ) {
		continue;
	}
	$line  = fgets( $conn, 8192 );
	$line  = $line === false ? '' : trim( $line );
	$parts = preg_split( '/\s+/', $line ) ?: array();
	$token = $parts ? (string) end( $parts ) : '';

	if ( ! $accept_auth || ! hash_equals( $expected_token, $token ) ) {
		$resp = array(
			'ok'             => false,
			'command'        => 'purge',
			'site_id'        => 0,
			'purged_entries' => 0,
			'purged_bytes'   => 0,
			'generation'     => 0,
			'error'          => 'unauthenticated',
		);
	} else {
		$cmd = $parts[0] ?? 'purge';
		$op  = $parts[1] ?? 'url';
		$resp = array(
			'ok'             => true,
			'command'        => $cmd . '.' . $op,
			'site_id'        => 42,
			'purged_entries' => 1,
			'purged_bytes'   => 10,
			'generation'     => 1,
		);
	}
	fwrite( $conn, json_encode( $resp ) . "\n" );
	fclose( $conn );
}

fclose( $server );
@unlink( $sock_path );
exit( 0 );
