/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
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
