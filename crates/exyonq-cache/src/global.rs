use crate::{ResponseCache, Singleflight};
use std::sync::LazyLock;

#[derive(Default)]
pub struct GlobalCache {
    pub cache: ResponseCache,
    pub singleflight: Singleflight,
}

static GLOBAL: LazyLock<GlobalCache> = LazyLock::new(GlobalCache::default);

pub fn global_response_cache() -> &'static ResponseCache {
    &GLOBAL.cache
}

pub fn global_singleflight() -> &'static Singleflight {
    &GLOBAL.singleflight
}

pub fn reset_global_for_tests() {
    GLOBAL.cache.clear();
    GLOBAL.singleflight.clear();
    crate::metrics::reset_metrics_for_tests();
    #[cfg(any(debug_assertions, feature = "test-utils"))]
    crate::singleflight::set_singleflight_follower_hook_for_tests(None);
}
