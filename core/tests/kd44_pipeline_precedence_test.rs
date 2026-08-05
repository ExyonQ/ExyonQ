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
