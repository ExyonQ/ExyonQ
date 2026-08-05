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
//! Mechanical adapter — structural route rules (KD4.3).

use crate::config::RouteConfig;
use exyonq_module_api::route_rules::{
    evaluate_structural_route_rules, RouteRuleInput, RouteRuleOutcome,
};

#[inline]
pub fn structural_route_rule_input(route: &RouteConfig) -> RouteRuleInput<'_> {
    RouteRuleInput {
        redirect_status: route.redirect.as_ref().map(|r| r.status),
        redirect_location: route.redirect.as_ref().map(|r| r.location.as_str()),
        rewrite_target: route.rewrite.as_deref(),
    }
}

#[inline]
pub fn evaluate_route_structural_rules(route: &RouteConfig) -> RouteRuleOutcome<'_> {
    evaluate_structural_route_rules(structural_route_rule_input(route))
}
