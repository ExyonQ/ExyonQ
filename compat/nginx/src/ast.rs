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
//! Minimal NGINX configuration AST.

use crate::report::SourceLoc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub directives: Vec<Directive>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directive {
    pub name: String,
    pub args: Vec<String>,
    pub block: Option<Block>,
    pub loc: SourceLoc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub directives: Vec<Directive>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationKind {
    Prefix,
    Exact,
    PreferentialPrefix,
    RegexCaseSensitive,
    RegexCaseInsensitive,
    Named,
}

impl LocationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LocationKind::Prefix => "Prefix",
            LocationKind::Exact => "Exact",
            LocationKind::PreferentialPrefix => "PreferentialPrefix",
            LocationKind::RegexCaseSensitive => "RegexCaseSensitive",
            LocationKind::RegexCaseInsensitive => "RegexCaseInsensitive",
            LocationKind::Named => "Named",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLocation {
    pub kind: LocationKind,
    pub raw_pattern: String,
    pub normalized_literal_path: Option<String>,
    pub order: usize,
    pub directives: Vec<Directive>,
    pub loc: SourceLoc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedServer {
    pub directives: Vec<Directive>,
    pub locations: Vec<ParsedLocation>,
    pub loc: SourceLoc,
    /// NG-18 / ADR-064: server-scope `gunzip on|off`.
    pub gunzip: Option<bool>,
    /// NG-18: server-scope `gunzip_buffers` total bytes.
    pub gunzip_buffers: Option<usize>,
    /// NG-18 Phase C: server-scope `sub_filter` policy.
    pub sub_filter: Option<ParsedSubFilter>,
    /// NG-18 Phase D: server-scope `mirror` policy.
    pub mirror: Option<ParsedMirror>,
    /// NG-18 Phase E: server-scope `slice` size.
    pub slice: Option<ParsedSlice>,
    /// 0.5: server-scope `auth_basic` policy.
    pub auth: Option<ParsedAuth>,
    /// 0.5: server-scope `limit_req` default.
    pub limit_req: Option<ParsedLimitReq>,
}

/// NG-18 Phase C: nginx `sub_filter*` directives at one scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedSubFilter {
    pub rules: Vec<exyonq_config_ir::SubFilterRule>,
    pub once: Option<bool>,
    pub types: Option<Vec<String>>,
    pub last_modified: Option<bool>,
}

/// NG-18 Phase D: nginx `mirror` directives at one scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedMirror {
    pub destinations: Vec<String>,
    pub request_body: Option<bool>,
    /// `mirror off`: explicitly disables inherited destinations at this scope.
    pub off: bool,
}

/// NG-18 Phase E: nginx `slice <size>` at one scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedSlice {
    pub size: usize,
}

/// 0.5: nginx `auth_basic` / `auth_basic_user_file` at one scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedAuth {
    pub realm: Option<String>,
    pub user_file: Option<String>,
    pub off: bool,
}

/// 0.5: nginx `limit_req_zone`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLimitZone {
    pub key: String,
    pub requests_per_second: u32,
}

/// 0.5: nginx `limit_req`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLimitReq {
    pub zone: String,
    pub burst: u32,
}

/// NG-07: nginx upstream load-balancing method (`least_conn`, `ip_hash`,
/// `hash … consistent`, `sticky cookie <name>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NginxLbMethod {
    LeastConn,
    IpHash,
    ConsistentHash,
    StickyCookie,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUpstream {
    pub name: String,
    /// Raw `server` lines — first whitespace token is the endpoint; later
    /// tokens are flags (`weight=N`, `backup`, `down`, `max_fails`, …).
    pub servers: Vec<String>,
    /// NG-07: at most one LB method per block (nginx rejects duplicates).
    pub method: Option<NginxLbMethod>,
    /// NG-07: cookie name from `sticky cookie <name>` (nginx-plus directive).
    pub sticky_cookie: Option<String>,
    /// NG-09: `resolver ... valid=T` re-resolve interval (ms); resolver
    /// addresses are ignored — ExyonQ resolves via system DNS.
    pub dns_refresh_ms: Option<u64>,
    pub loc: SourceLoc,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnalyzedConfig {
    pub servers: Vec<ParsedServer>,
    pub upstreams: Vec<ParsedUpstream>,
    /// NG-05 / ADR-051: parsed `map $src $dst { … }` blocks.
    pub maps: Vec<ParsedMap>,
    /// NG-11: accumulated `set_real_ip_from` CIDRs across http/server contexts.
    pub real_ip_trusted: Vec<String>,
    /// NG-11: last `real_ip_header` seen wins (nginx innermost-wins ≈ last).
    pub real_ip_header: Option<String>,
    /// NG-18 / ADR-064: http-scope `gunzip on|off`.
    pub gunzip_http: Option<bool>,
    /// NG-18: http-scope `gunzip_buffers` total bytes.
    pub gunzip_buffers_http: Option<usize>,
    /// NG-18 Phase C: http-scope `sub_filter` policy.
    pub sub_filter_http: Option<ParsedSubFilter>,
    /// NG-18 Phase D: http-scope `mirror` policy.
    pub mirror_http: Option<ParsedMirror>,
    /// NG-18 Phase E: http-scope `slice` size.
    pub slice_http: Option<ParsedSlice>,
    /// 0.5: http-scope `auth_basic` policy.
    pub auth_http: Option<ParsedAuth>,
    /// 0.5: http-scope `limit_req_zone` definitions.
    pub limit_zones_http: std::collections::BTreeMap<String, ParsedLimitZone>,
    /// 0.5: http-scope `limit_req` default.
    pub limit_req_http: Option<ParsedLimitReq>,
}

/// Nginx `map` block reduced to IR-friendly fields (Exact only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMap {
    pub loc: crate::report::SourceLoc,
    pub source_var: String,
    pub dest_var: String,
    pub default: String,
    pub values: std::collections::HashMap<String, String>,
}
