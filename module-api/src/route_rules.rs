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
//! Structural route redirect/rewrite evaluation (KD4.3).
//!
//! Compiled config metadata only — no Hyper, Tokio, or snapshot types.

/// Frozen dispatch stage ids (KD4.3). Changing order is an architecture regression.
pub const DISPATCH_STAGE_ROUTE_LOOKUP: u8 = 1;
pub const DISPATCH_STAGE_HTACCESS_OVERLAY: u8 = 2;
pub const DISPATCH_STAGE_STRUCTURAL_RULES: u8 = 3;
pub const DISPATCH_STAGE_BACKEND: u8 = 4;

/// Canonical request-path evaluation order (overlay before structural rules — frozen v0).
pub const FROZEN_DISPATCH_STAGE_ORDER: [u8; 4] = [
    DISPATCH_STAGE_ROUTE_LOOKUP,
    DISPATCH_STAGE_HTACCESS_OVERLAY,
    DISPATCH_STAGE_STRUCTURAL_RULES,
    DISPATCH_STAGE_BACKEND,
];

/// Compiled structural rule metadata for one route (reload-time immutable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteRuleInput<'a> {
    pub redirect_status: Option<u16>,
    pub redirect_location: Option<&'a str>,
    pub rewrite_target: Option<&'a str>,
}

/// Outcome of structural redirect/rewrite evaluation — no socket/response construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteRuleOutcome<'a> {
    NoChange,
    Redirect {
        status: u16,
        location: &'a str,
    },
    InternalRewrite {
        path: &'a str,
    },
    /// Location rejected at runtime (CRLF / control chars in `Location`).
    RejectedRedirect,
}

#[inline]
fn location_rejected(location: &str) -> bool {
    location
        .as_bytes()
        .iter()
        .any(|&b| b == b'\r' || b == b'\n')
}

/// Evaluate structural redirect/rewrite for a matched route.
///
/// Priority when both are present matches compile-time validation: redirect wins.
/// `NoChange` returns without heap allocation.
#[inline]
pub fn evaluate_structural_route_rules(input: RouteRuleInput<'_>) -> RouteRuleOutcome<'_> {
    if let (Some(status), Some(location)) = (input.redirect_status, input.redirect_location) {
        if location.is_empty() || location_rejected(location) {
            return RouteRuleOutcome::RejectedRedirect;
        }
        return RouteRuleOutcome::Redirect { status, location };
    }
    if let Some(path) = input.rewrite_target {
        if path.is_empty() || !path.starts_with('/') {
            return RouteRuleOutcome::NoChange;
        }
        return RouteRuleOutcome::InternalRewrite { path };
    }
    RouteRuleOutcome::NoChange
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_dispatch_order_is_canonical() {
        assert_eq!(
            FROZEN_DISPATCH_STAGE_ORDER,
            [
                DISPATCH_STAGE_ROUTE_LOOKUP,
                DISPATCH_STAGE_HTACCESS_OVERLAY,
                DISPATCH_STAGE_STRUCTURAL_RULES,
                DISPATCH_STAGE_BACKEND,
            ]
        );
    }

    #[test]
    fn no_rule_is_no_change() {
        let input = RouteRuleInput {
            redirect_status: None,
            redirect_location: None,
            rewrite_target: None,
        };
        assert_eq!(
            evaluate_structural_route_rules(input),
            RouteRuleOutcome::NoChange
        );
    }

    #[test]
    fn redirect_permanent_and_temporary_statuses() {
        for status in [301_u16, 302, 303, 307, 308] {
            let input = RouteRuleInput {
                redirect_status: Some(status),
                redirect_location: Some("/dest"),
                rewrite_target: None,
            };
            assert_eq!(
                evaluate_structural_route_rules(input),
                RouteRuleOutcome::Redirect {
                    status,
                    location: "/dest"
                }
            );
        }
    }

    #[test]
    fn redirect_absolute_and_relative_location() {
        let abs = RouteRuleInput {
            redirect_status: Some(302),
            redirect_location: Some("https://example.com/new"),
            rewrite_target: None,
        };
        assert!(matches!(
            evaluate_structural_route_rules(abs),
            RouteRuleOutcome::Redirect { .. }
        ));
        let rel = RouteRuleInput {
            redirect_status: Some(301),
            redirect_location: Some("/new"),
            rewrite_target: None,
        };
        assert_eq!(
            evaluate_structural_route_rules(rel),
            RouteRuleOutcome::Redirect {
                status: 301,
                location: "/new"
            }
        );
    }

    #[test]
    fn redirect_crlf_rejected() {
        let input = RouteRuleInput {
            redirect_status: Some(302),
            redirect_location: Some("/bad\r\nInjected: x"),
            rewrite_target: None,
        };
        assert_eq!(
            evaluate_structural_route_rules(input),
            RouteRuleOutcome::RejectedRedirect
        );
    }

    #[test]
    fn internal_rewrite() {
        let input = RouteRuleInput {
            redirect_status: None,
            redirect_location: None,
            rewrite_target: Some("/index.html"),
        };
        assert_eq!(
            evaluate_structural_route_rules(input),
            RouteRuleOutcome::InternalRewrite {
                path: "/index.html"
            }
        );
    }

    #[test]
    fn redirect_precedes_rewrite_when_both_present() {
        let input = RouteRuleInput {
            redirect_status: Some(301),
            redirect_location: Some("/new"),
            rewrite_target: Some("/index.html"),
        };
        assert_eq!(
            evaluate_structural_route_rules(input),
            RouteRuleOutcome::Redirect {
                status: 301,
                location: "/new"
            }
        );
    }

    #[test]
    fn empty_rewrite_is_no_change() {
        let input = RouteRuleInput {
            redirect_status: None,
            redirect_location: None,
            rewrite_target: Some(""),
        };
        assert_eq!(
            evaluate_structural_route_rules(input),
            RouteRuleOutcome::NoChange
        );
    }

    #[test]
    fn rewrite_must_be_absolute_path() {
        let input = RouteRuleInput {
            redirect_status: None,
            redirect_location: None,
            rewrite_target: Some("relative"),
        };
        assert_eq!(
            evaluate_structural_route_rules(input),
            RouteRuleOutcome::NoChange
        );
    }
}
