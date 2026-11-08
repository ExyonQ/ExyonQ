=== ExyonQ Cache ===
Contributors: exyonq
Tags: cache, purge, performance, exyonq
Requires at least: 6.0
Tested up to: 6.7
Requires PHP: 8.0
Stable tag: 0.2.1
License: Apache-2.0
License URI: https://www.apache.org/licenses/LICENSE-2.0

Minimal WordPress invalidation client for ExyonQ L1 full-page cache (Unix purge socket).

== Description ==

ExyonQ Cache is an **invalidation client only**. It sends authenticated purge
commands to ExyonQ's WC3 Unix purge socket when WordPress content changes.

It does not store HTML, talk to Redis/Lux, or bypass ExyonQ's cache engine.

ExyonQ publishes `cache-purge.token` and `<socket>.sites` next to the purge
socket; the plugin reads them automatically. `wp-config.php` constants remain
an optional override for standalone deployments. The panel under **ExyonQ**
shows the resolved connection without printing the token.

Multisite is not supported in this version.

== Installation ==

1. Copy this directory to `wp-content/plugins/exyonq-cache/`.
2. Optional: define `EXYONQ_CACHE_PURGE_SOCKET`, `EXYONQ_CACHE_PURGE_TOKEN`, and
   `EXYONQ_CACHE_SITE_ID` in `wp-config.php` when the sidecars are not visible.
3. Ensure ExyonQ is running with the matching purge socket and token.
4. Activate the plugin (single-site).

== Frequently Asked Questions ==

= Where is the token stored? =

Only in a PHP `define()` (typically `wp-config.php`). Never in the database.

= What if ExyonQ is down? =

WordPress continues normally. Purge failures are non-fatal.

== Changelog ==

= 0.2.1 =
* Restore the ExyonQ admin panel (status, diagnostics, purge button).
* Read the u64 site id from `<socket>.sites`; a stale constant no longer wins.
* Read `cache-purge.token` and discover per-unit `/run/exyonq*/cache-purge.sock`.
* Show unknown server profile instead of claiming page caching is off.

= 0.1.0 =
* Initial WC4 minimal purge client.
