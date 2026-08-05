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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUpstream {
    pub name: String,
    pub servers: Vec<String>,
    pub loc: SourceLoc,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnalyzedConfig {
    pub servers: Vec<ParsedServer>,
    pub upstreams: Vec<ParsedUpstream>,
}
