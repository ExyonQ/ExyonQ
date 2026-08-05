=== ExyonQ Cache ===
Contributors: exyonq
Tags: cache, purge, performance, exyonq
Requires at least: 6.0
Tested up to: 6.7
Requires PHP: 8.0
Stable tag: 0.1.0
License: Apache-2.0
License URI: https://www.apache.org/licenses/LICENSE-2.0

Minimal WordPress invalidation client for ExyonQ L1 full-page cache (Unix purge socket).

== Description ==

ExyonQ Cache is an **invalidation client only**. It sends authenticated purge
commands to ExyonQ's WC3 Unix purge socket when WordPress content changes.

It does not store HTML, talk to Redis/Lux, or bypass ExyonQ's cache engine.

Configure socket, token, and site id via `wp-config.php` constants only.

Multisite is not supported in this version.

== Installation ==

1. Copy this directory to `wp-content/plugins/exyonq-cache/`.
2. Define `EXYONQ_CACHE_PURGE_SOCKET`, `EXYONQ_CACHE_PURGE_TOKEN`, and `EXYONQ_CACHE_SITE_ID` in `wp-config.php`.
3. Ensure ExyonQ is running with the matching purge socket and token.
4. Activate the plugin (single-site).

== Frequently Asked Questions ==

= Where is the token stored? =

Only in a PHP `define()` (typically `wp-config.php`). Never in the database.

= What if ExyonQ is down? =

WordPress continues normally. Purge failures are non-fatal.

== Changelog ==

= 0.1.0 =
* Initial WC4 minimal purge client.
