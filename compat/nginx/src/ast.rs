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
