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
//! KD4.4 — frozen cross-cutting pipeline order regression guard.

use exyonq_module_api::{
    FROZEN_PIPELINE_AROUND_CORE_ORDER, PIPELINE_STAGE_CORE_DISPATCH,
    PIPELINE_STAGE_REQUEST_FILTERS, PIPELINE_STAGE_RESPONSE_FILTERS,
};

#[test]
fn frozen_pipeline_around_core_order_regression_guard() {
    assert_eq!(
        FROZEN_PIPELINE_AROUND_CORE_ORDER,
        [
            PIPELINE_STAGE_REQUEST_FILTERS,
            PIPELINE_STAGE_CORE_DISPATCH,
            PIPELINE_STAGE_RESPONSE_FILTERS,
        ]
    );
}

#[test]
fn rate_limit_and_request_filters_precede_core_dispatch() {
    assert_eq!(
        FROZEN_PIPELINE_AROUND_CORE_ORDER[0],
        PIPELINE_STAGE_REQUEST_FILTERS
    );
    assert_eq!(
        FROZEN_PIPELINE_AROUND_CORE_ORDER[1],
        PIPELINE_STAGE_CORE_DISPATCH
    );
}

#[test]
fn compression_response_filters_follow_core_dispatch() {
    assert_eq!(
        FROZEN_PIPELINE_AROUND_CORE_ORDER[2],
        PIPELINE_STAGE_RESPONSE_FILTERS
    );
}
