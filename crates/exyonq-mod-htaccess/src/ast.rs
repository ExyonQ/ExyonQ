//! Parsed `.htaccess` AST (offline only).

use crate::report::SourceLoc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFile {
    pub path: String,
    pub directives: Vec<ParsedDirective>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDirective {
    pub name: String,
    pub args: Vec<String>,
    pub loc: SourceLoc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectiveSupport {
    Executed,
    ParsedOnly,
    Unknown,
}
