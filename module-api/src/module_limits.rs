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
//! #86: observability for `[modules] response_body_limit_bytes`.
//!
//! The counter is process-lifetime (like the other wire/kernel counters): a
//! reload may change the ceiling, but a hit already happened.

use std::sync::atomic::{AtomicU64, Ordering};

static MODULE_BODY_LIMIT_HITS: AtomicU64 = AtomicU64::new(0);

/// Count one response whose collected body exceeded the configured ceiling.
pub fn record_module_body_limit_hit() {
    MODULE_BODY_LIMIT_HITS.fetch_add(1, Ordering::Relaxed);
}

/// Responses rejected because the module chain body ceiling was exceeded.
pub fn module_body_limit_hits_total() -> u64 {
    MODULE_BODY_LIMIT_HITS.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_is_monotonic() {
        let before = module_body_limit_hits_total();
        record_module_body_limit_hit();
        assert_eq!(module_body_limit_hits_total(), before + 1);
    }
}
