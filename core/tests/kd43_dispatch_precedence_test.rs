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
