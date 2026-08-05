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
//! Cross-cutting pipeline contract and frozen stage order (KD4.4).

/// Route lookup + structural rules + overlay + backend dispatch (core-owned stages).
pub const PIPELINE_STAGE_CORE_DISPATCH: u8 = 1;
/// Request filters (`on_request`) and route short-circuits (`on_route`) before core dispatch.
pub const PIPELINE_STAGE_REQUEST_FILTERS: u8 = 2;
/// Response filters (`on_response`) after core dispatch (compression, metrics recording).
pub const PIPELINE_STAGE_RESPONSE_FILTERS: u8 = 3;

/// Frozen relative order around core dispatch (v0 — do not reorder without ADR).
///
/// ```text
/// request filters / rate-limit short-circuit
/// → core dispatch (route, overlay, structural rules, cache, backend)
/// → response filters (compression)
/// ```
pub const FROZEN_PIPELINE_AROUND_CORE_ORDER: [u8; 3] = [
    PIPELINE_STAGE_REQUEST_FILTERS,
    PIPELINE_STAGE_CORE_DISPATCH,
    PIPELINE_STAGE_RESPONSE_FILTERS,
];

/// Compiled enable flags for cross-cutting modules (reload-time immutable metadata).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompiledPipelineFlags {
    pub metrics: bool,
    pub compression: bool,
    pub ratelimit: bool,
}

impl CompiledPipelineFlags {
    pub fn any_enabled(&self) -> bool {
        self.metrics || self.compression || self.ratelimit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_pipeline_order_regression_guard() {
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
    fn request_filters_precede_response_filters() {
        let req_pos = FROZEN_PIPELINE_AROUND_CORE_ORDER
            .iter()
            .position(|&s| s == PIPELINE_STAGE_REQUEST_FILTERS)
            .expect("request filters");
        let resp_pos = FROZEN_PIPELINE_AROUND_CORE_ORDER
            .iter()
            .position(|&s| s == PIPELINE_STAGE_RESPONSE_FILTERS)
            .expect("response filters");
        assert!(req_pos < resp_pos);
    }
}
