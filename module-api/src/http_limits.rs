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
//! NG-21: the committed [`tuning`](crate::runtime_tuning) geometry as the
//! header limits every protocol applies.
//!
//! HTTP/1.1 measures a request head in raw bytes; HTTP/2 and HTTP/3 deliver a
//! decoded header list, so the comparable measure is the one the `h2`
//! protocol itself uses: `name.len() + value.len() + 32` per field. Both use
//! the same configured budget, so a deployment that accepts a header block in
//! H1 does not silently accept a larger one in H2 or H3.
//!
//! The values come from the cell core publishes for the committed generation;
//! with no `[tuning]` table they fall back to the historical 8192 bytes and the
//! 30 s deadline, which is what H1 does today.

use std::time::Duration;

use crate::runtime_tuning;

/// Per-field overhead the `h2` protocol adds to `name.len() + value.len()`.
pub const HEADER_FIELD_OVERHEAD: usize = 32;

/// Committed header budget in the H1 (`max_buf_size`) unit.
pub fn header_budget_bytes() -> usize {
    runtime_tuning::max_header_bytes()
}

/// Committed header deadline, the same one H1 applies to a request head.
///
/// H2/H3 read the same resolution H1 does, so no protocol ends up with a laxer
/// deadline than HTTP/1.1.
pub fn header_deadline() -> Duration {
    runtime_tuning::header_read_deadline()
}

/// Size of a decoded header list under the `h2` metric.
pub fn header_list_bytes(headers: &http::HeaderMap) -> usize {
    headers
        .iter()
        .map(|(name, value)| name.as_str().len() + value.as_bytes().len() + HEADER_FIELD_OVERHEAD)
        .sum()
}

/// True when the decoded header list fits the committed budget.
pub fn header_list_within_budget(headers: &http::HeaderMap) -> bool {
    header_list_bytes(headers) <= header_budget_bytes()
}

#[cfg(test)]
mod tests {
    use super::{
        header_budget_bytes, header_deadline, header_list_bytes, header_list_within_budget,
        HEADER_FIELD_OVERHEAD,
    };
    use crate::runtime_tuning::{publish, reset_for_tests};
    use std::time::Duration;

    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner())
    }

    #[test]
    fn falls_back_to_the_historical_geometry() {
        let _guard = lock();
        reset_for_tests();
        assert_eq!(header_budget_bytes(), 8192);
        assert_eq!(header_deadline(), Duration::from_millis(30_000));
    }

    #[test]
    fn follows_the_committed_geometry() {
        let _guard = lock();
        publish(Some(400), Some(16_384));
        assert_eq!(header_budget_bytes(), 16_384);
        assert_eq!(header_deadline(), Duration::from_millis(400));
        reset_for_tests();
    }

    #[test]
    fn the_metric_matches_the_h2_field_cost() {
        let _guard = lock();
        publish(None, Some(8192));
        let mut headers = http::HeaderMap::new();
        headers.insert("x-pad", http::HeaderValue::from_static("abc"));
        assert_eq!(
            header_list_bytes(&headers),
            "x-pad".len() + 3 + HEADER_FIELD_OVERHEAD
        );
        assert!(header_list_within_budget(&headers));

        let big = http::HeaderValue::from_str(&"a".repeat(8192)).unwrap();
        headers.insert("x-big", big);
        assert!(!header_list_within_budget(&headers));
        reset_for_tests();
    }
}
