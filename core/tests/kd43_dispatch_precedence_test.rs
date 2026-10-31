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
//! KD4.3 — frozen dispatch precedence between structural rules and overlay.

use exyonq_module_api::{
    DISPATCH_STAGE_BACKEND, DISPATCH_STAGE_HTACCESS_OVERLAY, DISPATCH_STAGE_ROUTE_LOOKUP,
    DISPATCH_STAGE_STRUCTURAL_RULES, FROZEN_DISPATCH_STAGE_ORDER,
};

const EXPECTED_ORDER: [u8; 4] = [
    DISPATCH_STAGE_ROUTE_LOOKUP,
    DISPATCH_STAGE_HTACCESS_OVERLAY,
    DISPATCH_STAGE_STRUCTURAL_RULES,
    DISPATCH_STAGE_BACKEND,
];

#[test]
fn frozen_dispatch_stage_order_regression_guard() {
    assert_eq!(
        FROZEN_DISPATCH_STAGE_ORDER, EXPECTED_ORDER,
        "dispatch precedence changed — update KD4.3 report and integration tests"
    );
}

#[test]
fn htaccess_overlay_stage_precedes_structural_rules() {
    let overlay_pos = FROZEN_DISPATCH_STAGE_ORDER
        .iter()
        .position(|&s| s == DISPATCH_STAGE_HTACCESS_OVERLAY)
        .expect("overlay stage");
    let structural_pos = FROZEN_DISPATCH_STAGE_ORDER
        .iter()
        .position(|&s| s == DISPATCH_STAGE_STRUCTURAL_RULES)
        .expect("structural stage");
    assert!(
        overlay_pos < structural_pos,
        "overlay must run before structural redirect/rewrite (frozen v0)"
    );
}
