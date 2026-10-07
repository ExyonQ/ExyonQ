<?php
/**
 * Admin screen: cache status, purge statistics, and a purge button.
 *
 * @package ExyonQ_Cache
 */

declare(strict_types=1);

if ( ! defined( 'ABSPATH' ) ) {
	exit;
}

final class ExyonQ_Cache_Admin {

	private ExyonQ_Cache_Plugin $plugin;

	public function __construct( ExyonQ_Cache_Plugin $plugin ) {
		$this->plugin = $plugin;
	}

	public static function register( ExyonQ_Cache_Plugin $plugin ): void {
		$admin = new self( $plugin );
		add_action( 'admin_menu', array( $admin, 'menu' ) );
		add_action( 'admin_bar_menu', array( $admin, 'admin_bar' ), 80 );
		add_action( 'admin_post_exyonq_cache_purge', array( $admin, 'handle_purge' ) );
		add_filter( 'plugin_action_links_' . plugin_basename( EXYONQ_CACHE_PLUGIN_FILE ), array( $admin, 'action_links' ) );
	}

	/**
	 * @param list<string> $links
	 * @return list<string>
	 */
	public function action_links( array $links ): array {
		$url = admin_url( 'admin.php?page=exyonq-cache' );
		array_unshift( $links, '<a href="' . esc_url( $url ) . '">Panel</a>' );
		return $links;
	}

	public function menu(): void {
		add_menu_page(
			'Caché ExyonQ',
			'ExyonQ',
			'manage_options',
			'exyonq-cache',
			array( $this, 'render' ),
			'dashicons-performance',
			58
		);
	}

	/**
	 * @param WP_Admin_Bar $bar Admin bar.
	 */
	public function admin_bar( $bar ): void {
		if ( ! current_user_can( 'manage_options' ) ) {
			return;
		}
		$bar->add_node(
			array(
				'id'    => 'exyonq-cache',
				'title' => 'Caché ExyonQ',
				'href'  => admin_url( 'admin.php?page=exyonq-cache' ),
			)
		);
	}

	public function handle_purge(): void {
		if ( ! current_user_can( 'manage_options' ) ) {
			wp_die( esc_html__( 'No tienes permiso para purgar la caché.', 'exyonq-cache' ) );
		}
		check_admin_referer( 'exyonq_cache_purge' );
		$result = $this->plugin->client()->purge_site();
		$flag   = ! empty( $result['ok'] ) ? 'ok' : 'fail';
		wp_safe_redirect(
			add_query_arg(
				array(
					'page'         => 'exyonq-cache',
					'exyonq_purge' => $flag,
				),
				admin_url( 'admin.php' )
			)
		);
		exit;
	}

	public function render(): void {
		if ( ! current_user_can( 'manage_options' ) ) {
			return;
		}
		$status  = $this->plugin->client()->public_status();
		$stats   = get_option( 'exyonq_cache_stats', array() );
		if ( ! is_array( $stats ) ) {
			$stats = array();
		}
		$profile = $this->read_profile();
		$notice  = isset( $_GET['exyonq_purge'] ) ? sanitize_key( wp_unslash( (string) $_GET['exyonq_purge'] ) ) : '';
		$last_ok = isset( $stats['last_ok'] ) ? (int) $stats['last_ok'] : -1;
		$last_at = isset( $stats['last_at'] ) ? (int) $stats['last_at'] : 0;
		$error   = isset( $stats['last_error'] ) ? (string) $stats['last_error'] : '';
		$this->styles();
		?>
		<div class="wrap exq-app">
			<header class="exq-hero">
				<div class="exq-hero-copy">
					<p class="exq-kicker">ExyonQ <?php echo esc_html( EXYONQ_CACHE_PLUGIN_VERSION ); ?></p>
					<h1>Caché de página</h1>
					<p>Las visitas anónimas se sirven desde la caché. Publicar, editar o borrar invalida esa copia. Este botón la borra ahora.</p>
				</div>
				<form method="post" action="<?php echo esc_url( admin_url( 'admin-post.php' ) ); ?>">
					<?php wp_nonce_field( 'exyonq_cache_purge' ); ?>
					<input type="hidden" name="action" value="exyonq_cache_purge" />
					<button type="submit" class="exq-purge">Purgar caché</button>
				</form>
			</header>

			<?php if ( $notice === 'ok' ) : ?>
				<div class="exq-note exq-note-ok" role="status">Caché purgada. ExyonQ eliminó <?php echo esc_html( (string) (int) ( $stats['last_entries'] ?? 0 ) ); ?> entradas (<?php echo esc_html( $this->format_bytes( (int) ( $stats['last_bytes'] ?? 0 ) ) ); ?>).</div>
			<?php elseif ( $notice === 'fail' ) : ?>
				<div class="exq-note exq-note-bad" role="alert"><?php echo esc_html( $this->error_label( $error ) ); ?></div>
			<?php endif; ?>

			<section class="exq-grid">
				<article class="exq-card">
					<p class="exq-label">Estado</p>
					<p class="exq-value"><?php echo $status['configured'] ? 'Lista' : 'Incompleta'; ?></p>
					<p class="exq-meta"><?php echo $profile['enabled'] ? 'Caché de página encendida' : 'Caché de página apagada en la configuración'; ?></p>
				</article>
				<article class="exq-card">
					<p class="exq-label">Duración</p>
					<p class="exq-value"><?php echo esc_html( (string) $profile['ttl'] ); ?> s</p>
					<p class="exq-meta">Una página anónima caduca sola pasado ese tiempo</p>
				</article>
				<article class="exq-card">
					<p class="exq-label">Última purga</p>
					<p class="exq-value exq-value-sm"><?php echo esc_html( $last_at > 0 ? wp_date( 'j M Y, H:i', $last_at ) : 'Ninguna' ); ?></p>
					<p class="exq-meta"><?php echo esc_html( $this->last_result_label( $last_ok, $error ) ); ?></p>
				</article>
				<article class="exq-card">
					<p class="exq-label">Purgas</p>
					<p class="exq-value"><?php echo esc_html( number_format_i18n( (int) ( $stats['success'] ?? 0 ) ) ); ?></p>
					<p class="exq-meta"><?php echo esc_html( $this->count_label( (int) ( $stats['empty'] ?? 0 ), 'sin copia', 'sin copia' ) ); ?> · <?php echo esc_html( $this->count_label( (int) ( $stats['failure'] ?? 0 ), 'fallida', 'fallidas' ) ); ?> · <?php echo esc_html( $this->count_label( (int) ( $stats['attempts'] ?? 0 ), 'intento', 'intentos' ) ); ?></p>
				</article>
			</section>

			<section class="exq-panel">
				<h2>Conexión con ExyonQ</h2>
				<dl class="exq-dl">
					<div><dt>Socket</dt><dd><code><?php echo esc_html( $status['socket'] !== '' ? $status['socket'] : '—' ); ?></code></dd></div>
					<div><dt>Socket presente</dt><dd><?php echo $status['socket_present'] ? 'Sí' : 'No'; ?></dd></div>
					<div><dt>Token</dt><dd><?php echo $status['token_present'] ? 'Presente (no se muestra)' : 'Ausente'; ?></dd></div>
					<div><dt>Sitio</dt><dd><?php echo esc_html( $status['site_id'] !== '' ? $status['site_id'] : 'Sin índice de sitio' ); ?></dd></div>
					<div><dt>Capacidad</dt><dd><?php echo esc_html( number_format_i18n( $profile['entries'] ) ); ?> entradas · <?php echo esc_html( $this->format_bytes( $profile['bytes'] ) ); ?></dd></div>
				</dl>
				<?php if ( $status['site_id'] === '' ) : ?>
					<p class="exq-hint">ExyonQ escribe el identificador del sitio junto al socket. Sin ese archivo el botón no puede purgar. Guardar una entrada en WordPress sigue funcionando.</p>
				<?php elseif ( $error === 'not_found' || $last_ok === 2 ) : ?>
					<p class="exq-hint">Las visitas con la sesión iniciada no se guardan. Si nadie anónimo ha abierto esa URL, ExyonQ responde que no hay copia. El botón «Purgar caché» vacía el sitio entero y esa orden sí se acepta.</p>
				<?php endif; ?>
			</section>
		</div>
		<?php
	}

	private function count_label( int $count, string $one, string $many ): string {
		$word = $count === 1 ? $one : $many;
		return number_format_i18n( $count ) . ' ' . $word;
	}

	private function last_result_label( int $last_ok, string $error ): string {
		if ( $last_ok < 0 ) {
			return 'Todavía no hay un resultado';
		}
		if ( $last_ok === 1 ) {
			return 'La última purga fue aceptada';
		}
		if ( $last_ok === 2 || $error === 'not_found' ) {
			return 'Esa página no estaba en la caché. No había nada que borrar.';
		}
		return $this->error_label( $error );
	}

	private function error_label( string $code ): string {
		$labels = array(
			'configuration_missing' => 'Falta el socket, el token o el identificador del sitio.',
			'socket_missing'        => 'ExyonQ no está escuchando en el socket de purga.',
			'connect_failed'        => 'No se pudo conectar con el socket de purga.',
			'timeout'               => 'ExyonQ no respondió a tiempo.',
			'unauthenticated'       => 'ExyonQ rechazó el token de purga.',
			'write_failed'          => 'No se pudo escribir en el socket de purga.',
			'empty_response'        => 'ExyonQ cerró la conexión sin responder.',
			'malformed_response'    => 'La respuesta de ExyonQ no es válida.',
			'response_too_large'    => 'La respuesta de ExyonQ es demasiado grande.',
			'not_found'             => 'Esa página no estaba en la caché. No había nada que borrar.',
			'invalid_key'           => 'Esa dirección lleva parámetros que la caché no guarda.',
			'rate_limited'          => 'Demasiadas purgas seguidas. Espera un momento y vuelve a intentarlo.',
			'unauthorized'          => 'El identificador del sitio no corresponde a esta caché.',
			'invalid_scope'         => 'El identificador del sitio no es válido.',
			'purge_failed'          => 'ExyonQ rechazó la purga.',
		);
		if ( isset( $labels[ $code ] ) ) {
			return $labels[ $code ];
		}
		return 'La purga no se completó.';
	}

	/**
	 * @return array{enabled:bool,ttl:int,entries:int,bytes:int}
	 */
	private function read_profile(): array {
		$profile = array(
			'enabled' => false,
			'ttl'     => 0,
			'entries' => 0,
			'bytes'   => 0,
		);
		$path = '/etc/exyonq/wordpress.toml';
		if ( ! is_readable( $path ) ) {
			return $profile;
		}
		$text = file_get_contents( $path );
		if ( ! is_string( $text ) ) {
			return $profile;
		}
		$part = $text;
		$at   = strpos( $text, '[full_page_cache]' );
		if ( $at !== false ) {
			$part = substr( $text, $at );
			$next = strpos( $part, "\n[" , 1 );
			if ( $next !== false ) {
				$part = substr( $part, 0, $next );
			}
		}
		if ( preg_match( '/^enabled\s*=\s*true\s*$/m', $part ) ) {
			$profile['enabled'] = true;
		}
		if ( preg_match( '/^default_ttl_seconds\s*=\s*(\d+)\s*$/m', $part, $m ) ) {
			$profile['ttl'] = (int) $m[1];
		}
		if ( preg_match( '/^max_entries\s*=\s*(\d+)\s*$/m', $part, $m ) ) {
			$profile['entries'] = (int) $m[1];
		}
		if ( preg_match( '/^max_total_bytes\s*=\s*(\d+)\s*$/m', $part, $m ) ) {
			$profile['bytes'] = (int) $m[1];
		}
		return $profile;
	}

	private function format_bytes( int $bytes ): string {
		if ( $bytes < 1024 ) {
			return number_format_i18n( $bytes ) . ' B';
		}
		if ( $bytes < 1048576 ) {
			return number_format_i18n( $bytes / 1024, 1 ) . ' KB';
		}
		return number_format_i18n( $bytes / 1048576, 1 ) . ' MB';
	}

	private function styles(): void {
		?>
		<style>
			.exq-app { max-width: 1080px; }
			.exq-hero {
				display: flex;
				justify-content: space-between;
				align-items: flex-end;
				gap: 24px;
				margin: 16px 0 18px;
				padding: 28px 32px;
				border-radius: 16px;
				background: linear-gradient(135deg, #0e1c2f 0%, #1a3a5c 100%);
				color: #f8fafc;
			}
			.exq-kicker {
				margin: 0 0 6px;
				letter-spacing: 0.14em;
				text-transform: uppercase;
				font-size: 12px;
				color: #93c5fd;
			}
			.exq-hero h1 { margin: 0 0 8px; color: #fff; font-size: 32px; font-weight: 650; }
			.exq-hero-copy p:last-child { margin: 0; max-width: 62ch; color: #dbe7f5; }
			.exq-purge {
				border: 0;
				border-radius: 999px;
				padding: 14px 28px;
				background: #fff;
				color: #0e1c2f;
				font-size: 15px;
				font-weight: 650;
				cursor: pointer;
			}
			.exq-purge:hover { background: #e0f2fe; }
			.exq-note { margin: 0 0 16px; padding: 12px 16px; border-radius: 10px; }
			.exq-note-ok { background: #ecfdf3; color: #166534; }
			.exq-note-bad { background: #fef2f2; color: #991b1b; }
			.exq-grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 12px; }
			.exq-card, .exq-panel {
				background: #fff;
				border: 1px solid #e2e8f0;
				border-radius: 14px;
				padding: 16px 18px;
			}
			.exq-label { margin: 0; color: #64748b; font-size: 12px; letter-spacing: 0.04em; text-transform: uppercase; }
			.exq-value { margin: 8px 0 4px; font-size: 28px; font-weight: 650; color: #0f172a; }
			.exq-value-sm { font-size: 18px; }
			.exq-meta { margin: 0; color: #475569; }
			.exq-panel { margin-top: 12px; }
			.exq-panel h2 { margin: 0 0 12px; font-size: 16px; }
			.exq-dl { display: grid; grid-template-columns: 1fr 1fr; gap: 10px 24px; margin: 0; }
			.exq-dl div { display: flex; justify-content: space-between; gap: 12px; padding-bottom: 8px; border-bottom: 1px solid #f1f5f9; }
			.exq-dl dt { color: #64748b; }
			.exq-dl dd { margin: 0; font-weight: 600; color: #0f172a; }
			.exq-hint { margin: 14px 0 0; color: #9a3412; }
			@media (max-width: 960px) {
				.exq-hero { flex-direction: column; align-items: flex-start; }
				.exq-grid, .exq-dl { grid-template-columns: 1fr; }
			}
		</style>
		<?php
	}
}
