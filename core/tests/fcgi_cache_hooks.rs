//! Register FastCGI cache hooks for integration tests (mirrors CLI composition root).

use exyonq_core::{
    fcgi_cache_hooks_from_module, register_fcgi_cache_hooks, FcgiCacheMetricsFns, ServeFn,
};
use exyonq_mod_fastcgi::{
    cache_fcgi_hits_total, cache_fcgi_insertions_total, cache_fcgi_misses_total,
    cache_fcgi_rejections_total, reset_fcgi_cache_metrics_for_tests, serve_fastcgi_with_cache_hook,
};
use std::sync::Once;

static INSTALL: Once = Once::new();

const SERVE_HOOK: ServeFn = serve_fastcgi_with_cache_hook;

pub fn ensure_fcgi_cache_hooks() {
    INSTALL.call_once(|| {
        let _ = register_fcgi_cache_hooks(fcgi_cache_hooks_from_module(
            SERVE_HOOK,
            FcgiCacheMetricsFns {
                hits: cache_fcgi_hits_total,
                misses: cache_fcgi_misses_total,
                insertions: cache_fcgi_insertions_total,
                rejections: cache_fcgi_rejections_total,
                reset: reset_fcgi_cache_metrics_for_tests,
            },
        ));
    });
}
