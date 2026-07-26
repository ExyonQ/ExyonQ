<?php
/**
 * WordPress hook wiring for ExyonQ cache invalidation.
 *
 * @package ExyonQ_Cache
 */

declare(strict_types=1);

if ( ! defined( 'ABSPATH' ) && ! defined( 'EXYONQ_CACHE_TEST_BOOTSTRAP' ) ) {
	exit;
}

final class ExyonQ_Cache_Hooks {

	private ExyonQ_Cache_Plugin $plugin;

	public function __construct( ExyonQ_Cache_Plugin $plugin ) {
		$this->plugin = $plugin;
	}

	public function register(): void {
		add_action( 'save_post', array( $this, 'on_save_post' ), 20, 3 );
		add_action( 'deleted_post', array( $this, 'on_deleted_post' ), 20, 1 );
		add_action( 'trashed_post', array( $this, 'on_trashed_post' ), 20, 1 );
		add_action( 'untrashed_post', array( $this, 'on_untrashed_post' ), 20, 1 );
		add_action( 'transition_post_status', array( $this, 'on_transition_post_status' ), 20, 3 );

		add_action( 'comment_post', array( $this, 'on_comment_change' ), 20, 1 );
		add_action( 'edit_comment', array( $this, 'on_comment_change' ), 20, 1 );
		add_action( 'deleted_comment', array( $this, 'on_comment_change' ), 20, 1 );
		add_action( 'wp_set_comment_status', array( $this, 'on_comment_status' ), 20, 2 );

		add_action( 'created_term', array( $this, 'on_term_change' ), 20, 3 );
		add_action( 'edited_term', array( $this, 'on_term_change' ), 20, 3 );
		add_action( 'delete_term', array( $this, 'on_term_delete' ), 20, 4 );

		add_action( 'switch_theme', array( $this, 'on_global_render_change' ), 20 );
		add_action( 'activated_plugin', array( $this, 'on_global_render_change' ), 20 );
		add_action( 'deactivated_plugin', array( $this, 'on_global_render_change' ), 20 );
		add_action( 'update_option_permalink_structure', array( $this, 'on_global_render_change' ), 20 );
	}

	/**
	 * @param int          $post_id Post ID.
	 * @param WP_Post|null $post    Post object.
	 * @param bool         $update  Whether this is an update.
	 */
	public function on_save_post( $post_id, $post = null, $update = false ): void {
		unset( $update );
		$post_id = (int) $post_id;
		if ( $post_id <= 0 || wp_is_post_revision( $post_id ) || wp_is_post_autosave( $post_id ) ) {
			return;
		}
		if ( ! $post instanceof WP_Post ) {
			$post = get_post( $post_id );
		}
		if ( ! $post instanceof WP_Post ) {
			return;
		}
		if ( $post->post_status !== 'publish' ) {
			return;
		}
		$this->purge_post_public( $post );
	}

	public function on_deleted_post( $post_id ): void {
		$post = get_post( (int) $post_id );
		if ( $post instanceof WP_Post ) {
			$this->purge_post_public( $post );
			return;
		}
		// Permanent delete may leave no resolvable permalink — bounded site fallback.
		$this->plugin->purge_site_once( 'deleted_post' );
	}

	public function on_trashed_post( $post_id ): void {
		$this->purge_post_id_public( (int) $post_id );
	}

	public function on_untrashed_post( $post_id ): void {
		$post = get_post( (int) $post_id );
		if ( $post instanceof WP_Post && $post->post_status === 'publish' ) {
			$this->purge_post_public( $post );
		}
	}

	/**
	 * @param string  $new_status New status.
	 * @param string  $old_status Old status.
	 * @param WP_Post $post       Post.
	 */
	public function on_transition_post_status( $new_status, $old_status, $post ): void {
		if ( ! $post instanceof WP_Post ) {
			return;
		}
		if ( wp_is_post_revision( $post->ID ) ) {
			return;
		}
		$was_public = ( $old_status === 'publish' );
		$is_public  = ( $new_status === 'publish' );
		if ( ! $was_public && ! $is_public ) {
			return;
		}
		// save_post also fires; dedupe handles duplicate URL purges.
		$this->purge_post_public( $post );
	}

	public function on_comment_change( $comment_id ): void {
		$comment = get_comment( $comment_id );
		if ( ! $comment ) {
			return;
		}
		$post = get_post( (int) $comment->comment_post_ID );
		if ( $post instanceof WP_Post && $post->post_status === 'publish' ) {
			$this->plugin->purge_urls( array( get_permalink( $post ) ) );
		}
	}

	/**
	 * @param int    $comment_id Comment ID.
	 * @param string $status     Status.
	 */
	public function on_comment_status( $comment_id, $status ): void {
		unset( $status );
		$this->on_comment_change( $comment_id );
	}

	/**
	 * @param int    $term_id  Term ID.
	 * @param int    $tt_id    Term taxonomy ID.
	 * @param string $taxonomy Taxonomy.
	 */
	public function on_term_change( $term_id, $tt_id, $taxonomy ): void {
		unset( $tt_id );
		$link = get_term_link( (int) $term_id, (string) $taxonomy );
		if ( is_wp_error( $link ) || ! is_string( $link ) ) {
			$this->plugin->purge_site_once( 'term' );
			return;
		}
		$this->plugin->purge_urls( array( $link ) );
	}

	/**
	 * @param int    $term_id  Term ID.
	 * @param int    $tt_id    Term taxonomy ID.
	 * @param string $taxonomy Taxonomy.
	 * @param mixed  $deleted  Deleted term object.
	 */
	public function on_term_delete( $term_id, $tt_id, $taxonomy, $deleted ): void {
		unset( $term_id, $tt_id, $deleted );
		// Link may already be gone — conservative site purge.
		$this->plugin->purge_site_once( 'term_delete:' . (string) $taxonomy );
	}

	public function on_global_render_change(): void {
		// Theme/plugin/permalink changes can affect all rendered HTML.
		$this->plugin->purge_site_once( 'global_render' );
	}

	private function purge_post_id_public( int $post_id ): void {
		$post = get_post( $post_id );
		if ( $post instanceof WP_Post ) {
			$this->purge_post_public( $post );
		}
	}

	private function purge_post_public( WP_Post $post ): void {
		$urls = $this->collect_post_urls( $post );
		if ( $urls === null ) {
			$this->plugin->client()->note_fallback_site();
			$this->plugin->purge_site_once( 'post_unbounded' );
			return;
		}
		$this->plugin->purge_urls( $urls );
	}

	/**
	 * @return list<string>|null Null → site fallback.
	 */
	private function collect_post_urls( WP_Post $post ): ?array {
		$urls = array();
		$permalink = get_permalink( $post );
		if ( is_string( $permalink ) && $permalink !== '' ) {
			$urls[] = $permalink;
		}
		$urls[] = home_url( '/' );
		$page_for_posts = (int) get_option( 'page_for_posts' );
		if ( $page_for_posts > 0 ) {
			$link = get_permalink( $page_for_posts );
			if ( is_string( $link ) ) {
				$urls[] = $link;
			}
		}
		$archive = get_post_type_archive_link( $post->post_type );
		if ( is_string( $archive ) && $archive !== '' ) {
			$urls[] = $archive;
		}
		$author = get_author_posts_url( (int) $post->post_author );
		if ( is_string( $author ) && $author !== '' ) {
			$urls[] = $author;
		}
		$taxonomies = get_object_taxonomies( $post->post_type, 'names' );
		foreach ( $taxonomies as $taxonomy ) {
			$terms = get_the_terms( $post, $taxonomy );
			if ( ! is_array( $terms ) ) {
				continue;
			}
			foreach ( $terms as $term ) {
				$link = get_term_link( $term );
				if ( is_string( $link ) ) {
					$urls[] = $link;
				}
			}
		}
		if ( function_exists( 'get_post_comments_feed_link' ) ) {
			$feed = get_post_comments_feed_link( $post->ID );
			if ( is_string( $feed ) && $feed !== '' ) {
				$urls[] = $feed;
			}
		}

		$urls = array_values( array_unique( array_filter( $urls ) ) );
		if ( count( $urls ) > $this->plugin->max_urls_per_event() ) {
			return null;
		}
		return $urls;
	}
}
