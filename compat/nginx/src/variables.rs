//! NGINX variable reference classification (offline, no expansion).

use crate::limits::MAX_VARIABLE_REFERENCES_PER_DIRECTIVE;
use crate::report::{CompatStatus, CompatibilityReport, SourceLoc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariableClass {
    KnownPreservable,
    KnownUnsupported,
    RegexCapture,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableRef {
    pub name: String,
    pub class: VariableClass,
}

pub fn classify_variable(name: &str) -> VariableClass {
    match name {
        "$request_uri" => VariableClass::KnownPreservable,
        "$uri" | "$args" | "$query_string" | "$host" | "$scheme" => VariableClass::KnownUnsupported,
        s if s.starts_with('$') && s[1..].chars().all(|c| c.is_ascii_digit()) => {
            VariableClass::RegexCapture
        }
        s if s.starts_with('$') => VariableClass::Unknown,
        _ => VariableClass::Unknown,
    }
}

pub fn scan_variables(text: &str) -> Vec<VariableRef> {
    let mut out = Vec::new();
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let start = i;
            i += 1;
            while i < bytes.len() && is_var_char(bytes[i]) {
                i += 1;
            }
            let name = text[start..i].to_string();
            if !name.is_empty() {
                out.push(VariableRef {
                    class: classify_variable(&name),
                    name,
                });
            }
        } else {
            i += 1;
        }
    }
    out
}

fn is_var_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

pub fn report_variables(
    loc: &SourceLoc,
    directive: &str,
    text: &str,
    report: &mut CompatibilityReport,
) -> Vec<VariableRef> {
    let vars = scan_variables(text);
    if vars.len() > MAX_VARIABLE_REFERENCES_PER_DIRECTIVE {
        report.push(
            loc,
            directive,
            CompatStatus::Error,
            format!(
                "variable reference count exceeds limit ({MAX_VARIABLE_REFERENCES_PER_DIRECTIVE})"
            ),
            None,
        );
        return vars;
    }
    let blocking: Vec<_> = vars
        .iter()
        .filter(|v| !matches!(v.class, VariableClass::KnownPreservable))
        .map(|v| v.name.clone())
        .collect();
    if !blocking.is_empty() {
        report.push(
            loc,
            directive,
            CompatStatus::Partial,
            format!("variables block exact mapping: {}", blocking.join(", ")),
            None,
        );
    }
    vars
}
