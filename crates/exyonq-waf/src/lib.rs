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
//! Native ExyonQ WAF engine (WAF2+).
//!
//! Compiles config into an immutable [`CompiledWafSnapshot`], inspects via
//! [`exyonq_waf_api::WafEngine`]. WAF3 adds built-in `EXY-*` detectors and
//! budgeted normalization.

#![forbid(unsafe_code)]

mod challenge;
mod compile;
mod detect;
mod engine;
mod exclusion;
mod normalize;
mod snapshot;

pub use challenge::ChallengeService;
pub use compile::{
    compile_waf, BuiltinDetectors, CompileError, DetectorToggle, ExclusionInput, FailPolicy,
    IpFilterInput, MatchTarget, RuleInput, RulesetInput, WafCompileInput,
};
pub use engine::{FixedWafEngine, NativeWafEngine};
pub use normalize::NormalizeBudget;
pub use snapshot::{CompiledWafSnapshot, MapHeaders};

pub use exyonq_waf_api::{ChallengeIssue, ChallengeVerifyResult, WafChallengeHandler};

/// Rule id emitted when body inspection budget is exhausted under Block / LogAndAllow.
pub const RULE_ID_BODY_INSPECTION_LIMIT: &str = "EXY-LIMIT-BODY-001";
