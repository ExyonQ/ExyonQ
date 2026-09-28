//! Structured configuration diagnostics (P1.4-WS2).
//!
//! Lives in `exyonq-config-ir` so surface/merge/runtime-plan can consume without
//! depending on CLI or core.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable product diagnostic codes (initial freeze). Do not reuse numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticCode {
    /// EXY-CONFIG-0001
    UnsupportedConfigVersion,
    /// EXY-CONFIG-0002
    UnknownField,
    /// EXY-CONFIG-0003
    MissingRequiredField,
    /// EXY-CONFIG-0004
    InvalidValue,
    /// EXY-CONFIG-0005
    DuplicateDefinition,
    /// EXY-CONFIG-0006
    IncludeNotFound,
    /// EXY-CONFIG-0007
    IncludeCycle,
    /// EXY-CONFIG-0008
    MergeConflict,
    /// EXY-CONFIG-0009
    SurfaceParseError,
    /// EXY-CONFIG-0010
    SurfaceLoweringError,
    /// EXY-CONFIG-0011
    IrValidationError,
    /// EXY-CONFIG-0012
    RuntimePlanCompileError,
    /// EXY-CONFIG-0013
    SecretSourceError,
    /// EXY-CONFIG-0014
    UnsupportedSurfaceConstruct,
    /// EXY-CONFIG-0015
    DeprecatedField,
    /// EXY-RELOAD-0001
    ReloadParseRejected,
    /// EXY-RELOAD-0002
    ReloadValidationRejected,
    /// EXY-RELOAD-0003
    ReloadCompileRejected,
    /// EXY-RELOAD-0004
    ReloadPrepareFailed,
    /// EXY-RELOAD-0005
    ReloadListenerRestartRequired,
    /// EXY-RELOAD-0006
    ReloadPublishFailed,
    /// EXY-RELOAD-0007
    ReloadRetireTimeout,
    /// EXY-RELOAD-0008
    ReloadIdenticalNoOp,
    /// EXY-RELOAD-0009 — generation skew **or** live PATH ≠ daemon-bound `EXYONQ_CONFIG`.
    ReloadGenerationMismatch,
    /// EXY-RELOAD-0010
    ReloadAlreadyInProgress,
    /// EXY-IMPORT-0001
    ImportInvalidSourceSyntax,
    /// EXY-IMPORT-0002
    ImportUnsupportedDirective,
    /// EXY-IMPORT-0003
    ImportLossyTranslation,
    /// EXY-IMPORT-0004
    ImportUnsafeRejected,
    /// EXY-IMPORT-0005 — RESERVED_UNUSED until typed context mismatch emit exists.
    ImportContextMismatch,
    /// EXY-IMPORT-0006
    ImportTargetValidationFailed,
    /// EXY-IMPORT-0007
    ImportTargetCompileFailed,
    /// EXY-HTACCESS-0001
    HtaccessParseError,
    /// EXY-HTACCESS-0002
    HtaccessUnsupportedDirective,
    /// EXY-HTACCESS-0003
    HtaccessForbiddenDirective,
    /// EXY-HTACCESS-0004
    HtaccessCompileFailed,
    /// EXY-HTACCESS-0005
    HtaccessPreviousOverlayRetained,
    /// EXY-HTACCESS-0006
    HtaccessSiteNotFound,
}

impl Serialize for DiagnosticCode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DiagnosticCode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse_stable(&s).ok_or_else(|| serde::de::Error::custom(format!("unknown code {s}")))
    }
}

impl DiagnosticCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedConfigVersion => "EXY-CONFIG-0001",
            Self::UnknownField => "EXY-CONFIG-0002",
            Self::MissingRequiredField => "EXY-CONFIG-0003",
            Self::InvalidValue => "EXY-CONFIG-0004",
            Self::DuplicateDefinition => "EXY-CONFIG-0005",
            Self::IncludeNotFound => "EXY-CONFIG-0006",
            Self::IncludeCycle => "EXY-CONFIG-0007",
            Self::MergeConflict => "EXY-CONFIG-0008",
            Self::SurfaceParseError => "EXY-CONFIG-0009",
            Self::SurfaceLoweringError => "EXY-CONFIG-0010",
            Self::IrValidationError => "EXY-CONFIG-0011",
            Self::RuntimePlanCompileError => "EXY-CONFIG-0012",
            Self::SecretSourceError => "EXY-CONFIG-0013",
            Self::UnsupportedSurfaceConstruct => "EXY-CONFIG-0014",
            Self::DeprecatedField => "EXY-CONFIG-0015",
            Self::ReloadParseRejected => "EXY-RELOAD-0001",
            Self::ReloadValidationRejected => "EXY-RELOAD-0002",
            Self::ReloadCompileRejected => "EXY-RELOAD-0003",
            Self::ReloadPrepareFailed => "EXY-RELOAD-0004",
            Self::ReloadListenerRestartRequired => "EXY-RELOAD-0005",
            Self::ReloadPublishFailed => "EXY-RELOAD-0006",
            Self::ReloadRetireTimeout => "EXY-RELOAD-0007",
            Self::ReloadIdenticalNoOp => "EXY-RELOAD-0008",
            Self::ReloadGenerationMismatch => "EXY-RELOAD-0009",
            Self::ReloadAlreadyInProgress => "EXY-RELOAD-0010",
            Self::ImportInvalidSourceSyntax => "EXY-IMPORT-0001",
            Self::ImportUnsupportedDirective => "EXY-IMPORT-0002",
            Self::ImportLossyTranslation => "EXY-IMPORT-0003",
            Self::ImportUnsafeRejected => "EXY-IMPORT-0004",
            Self::ImportContextMismatch => "EXY-IMPORT-0005",
            Self::ImportTargetValidationFailed => "EXY-IMPORT-0006",
            Self::ImportTargetCompileFailed => "EXY-IMPORT-0007",
            Self::HtaccessParseError => "EXY-HTACCESS-0001",
            Self::HtaccessUnsupportedDirective => "EXY-HTACCESS-0002",
            Self::HtaccessForbiddenDirective => "EXY-HTACCESS-0003",
            Self::HtaccessCompileFailed => "EXY-HTACCESS-0004",
            Self::HtaccessPreviousOverlayRetained => "EXY-HTACCESS-0005",
            Self::HtaccessSiteNotFound => "EXY-HTACCESS-0006",
        }
    }

    pub fn parse_stable(s: &str) -> Option<Self> {
        Some(match s {
            "EXY-CONFIG-0001" => Self::UnsupportedConfigVersion,
            "EXY-CONFIG-0002" => Self::UnknownField,
            "EXY-CONFIG-0003" => Self::MissingRequiredField,
            "EXY-CONFIG-0004" => Self::InvalidValue,
            "EXY-CONFIG-0005" => Self::DuplicateDefinition,
            "EXY-CONFIG-0006" => Self::IncludeNotFound,
            "EXY-CONFIG-0007" => Self::IncludeCycle,
            "EXY-CONFIG-0008" => Self::MergeConflict,
            "EXY-CONFIG-0009" => Self::SurfaceParseError,
            "EXY-CONFIG-0010" => Self::SurfaceLoweringError,
            "EXY-CONFIG-0011" => Self::IrValidationError,
            "EXY-CONFIG-0012" => Self::RuntimePlanCompileError,
            "EXY-CONFIG-0013" => Self::SecretSourceError,
            "EXY-CONFIG-0014" => Self::UnsupportedSurfaceConstruct,
            "EXY-CONFIG-0015" => Self::DeprecatedField,
            "EXY-RELOAD-0001" => Self::ReloadParseRejected,
            "EXY-RELOAD-0002" => Self::ReloadValidationRejected,
            "EXY-RELOAD-0003" => Self::ReloadCompileRejected,
            "EXY-RELOAD-0004" => Self::ReloadPrepareFailed,
            "EXY-RELOAD-0005" => Self::ReloadListenerRestartRequired,
            "EXY-RELOAD-0006" => Self::ReloadPublishFailed,
            "EXY-RELOAD-0007" => Self::ReloadRetireTimeout,
            "EXY-RELOAD-0008" => Self::ReloadIdenticalNoOp,
            "EXY-RELOAD-0009" => Self::ReloadGenerationMismatch,
            "EXY-RELOAD-0010" => Self::ReloadAlreadyInProgress,
            "EXY-IMPORT-0001" => Self::ImportInvalidSourceSyntax,
            "EXY-IMPORT-0002" => Self::ImportUnsupportedDirective,
            "EXY-IMPORT-0003" => Self::ImportLossyTranslation,
            "EXY-IMPORT-0004" => Self::ImportUnsafeRejected,
            "EXY-IMPORT-0005" => Self::ImportContextMismatch,
            "EXY-IMPORT-0006" => Self::ImportTargetValidationFailed,
            "EXY-IMPORT-0007" => Self::ImportTargetCompileFailed,
            "EXY-HTACCESS-0001" => Self::HtaccessParseError,
            "EXY-HTACCESS-0002" => Self::HtaccessUnsupportedDirective,
            "EXY-HTACCESS-0003" => Self::HtaccessForbiddenDirective,
            "EXY-HTACCESS-0004" => Self::HtaccessCompileFailed,
            "EXY-HTACCESS-0005" => Self::HtaccessPreviousOverlayRetained,
            "EXY-HTACCESS-0006" => Self::HtaccessSiteNotFound,
            _ => return None,
        })
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Error => f.write_str("error"),
            Self::Warning => f.write_str("warning"),
            Self::Info => f.write_str("info"),
        }
    }
}

/// How precise a source location is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanQuality {
    Exact,
    Approximate,
    SourceOnly,
    NoSourceLocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start: Position,
    pub end: Position,
    pub quality: SpanQuality,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub severity: Severity,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<SourceSpan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
}

impl Diagnostic {
    pub fn error(code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Error,
            message: message.into(),
            source: None,
            span: None,
            field: None,
            received: None,
            expected: None,
            cause: None,
            suggestion: None,
            documentation: None,
            related: Vec::new(),
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    pub fn with_span(mut self, span: SourceSpan) -> Self {
        self.span = Some(span);
        self
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    pub fn with_expected(mut self, expected: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    pub fn with_received(mut self, received: impl Into<String>) -> Self {
        self.received = Some(redact_secrets(&received.into()));
        self
    }

    pub fn with_cause(mut self, cause: impl Into<String>) -> Self {
        self.cause = Some(redact_secrets(&cause.into()));
        self
    }

    pub fn with_documentation(mut self, doc: impl Into<String>) -> Self {
        self.documentation = Some(doc.into());
        self
    }
}

/// Versioned JSON diagnostics envelope (schema_version = 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticDocument {
    pub schema_version: u32,
    pub diagnostics: Vec<Diagnostic>,
}

impl DiagnosticDocument {
    pub const SCHEMA_VERSION: u32 = 1;

    pub fn new(mut diagnostics: Vec<Diagnostic>) -> Self {
        sort_diagnostics(&mut diagnostics);
        Self {
            schema_version: Self::SCHEMA_VERSION,
            diagnostics,
        }
    }

    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Deterministic: source, then line, then code, then message.
pub fn sort_diagnostics(diags: &mut [Diagnostic]) {
    diags.sort_by(|a, b| {
        (
            a.source.as_deref().unwrap_or(""),
            a.span.as_ref().map(|s| s.start.line).unwrap_or(0),
            a.span.as_ref().map(|s| s.start.column).unwrap_or(0),
            a.code.as_str(),
            a.message.as_str(),
        )
            .cmp(&(
                b.source.as_deref().unwrap_or(""),
                b.span.as_ref().map(|s| s.start.line).unwrap_or(0),
                b.span.as_ref().map(|s| s.start.column).unwrap_or(0),
                b.code.as_str(),
                b.message.as_str(),
            ))
    });
}

/// Human renderer (no ANSI required).
pub fn render_human(diags: &[Diagnostic]) -> String {
    let mut ordered = diags.to_vec();
    sort_diagnostics(&mut ordered);
    let mut out = String::new();
    for d in &ordered {
        out.push_str(&format!("{}[{}]: {}\n", d.severity, d.code, d.message));
        if let Some(src) = &d.source {
            if let Some(span) = &d.span {
                out.push_str(&format!(
                    "  --> {src}:{}:{}\n",
                    span.start.line, span.start.column
                ));
            } else {
                out.push_str(&format!("  --> {src}\n"));
            }
        }
        if let Some(field) = &d.field {
            out.push_str(&format!("   = field: {field}\n"));
        }
        if let Some(received) = &d.received {
            out.push_str(&format!("   = received: {received}\n"));
        }
        if let Some(expected) = &d.expected {
            out.push_str(&format!("   = expected: {expected}\n"));
        }
        if let Some(cause) = &d.cause {
            out.push_str(&format!("   = cause: {cause}\n"));
        }
        if let Some(suggestion) = &d.suggestion {
            out.push_str(&format!("   = suggestion: {suggestion}\n"));
        }
        if let Some(doc) = &d.documentation {
            out.push_str(&format!("   = documentation: {doc}\n"));
        }
        out.push('\n');
    }
    out
}

/// Convert byte offset into 1-based line/column (UTF-8 safe by char boundaries of `&str`).
pub fn offset_to_position(src: &str, offset: usize) -> Position {
    let offset = offset.min(src.len());
    let mut line = 1u32;
    let mut column = 1u32;
    for (i, ch) in src.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    Position { line, column }
}

pub fn span_from_byte_range(src: &str, range: std::ops::Range<usize>) -> SourceSpan {
    let start = offset_to_position(src, range.start);
    let end = offset_to_position(src, range.end);
    SourceSpan {
        start,
        end,
        quality: SpanQuality::Exact,
    }
}

/// Redact common secret patterns from diagnostic strings.
pub fn redact_secrets(input: &str) -> String {
    let mut out = input.to_string();
    for needle in [
        "password=",
        "Authorization:",
        "Bearer ",
        "BEGIN PRIVATE KEY",
        "BEGIN RSA PRIVATE KEY",
        "redis://:",
    ] {
        if let Some(idx) = out.find(needle) {
            let rest = &out[idx + needle.len()..];
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',')
                .unwrap_or(rest.len());
            out.replace_range(idx + needle.len()..idx + needle.len() + end, "[REDACTED]");
        }
    }
    // Absolute key file contents must never appear; redact suspiciously long hex blobs after key_file=
    if out.contains("key_file") && out.len() > 200 {
        return "[REDACTED: possible secret material]".to_string();
    }
    out
}

/// Closed-catalog typo suggestion (Levenshtein ≤ 2).
pub fn suggest_typo(received: &str, catalog: &[&str]) -> Option<String> {
    let mut best: Option<(&str, usize)> = None;
    for &cand in catalog {
        let d = edit_distance(received, cand);
        if d > 0 && d <= 2 && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((cand, d));
        }
    }
    best.map(|(c, _)| format!("replace `{received}` with `{c}`"))
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Known closed field catalogs for typo hints (root-level keys).
pub const ROOT_FIELD_CATALOG: &[&str] = &[
    "config_version",
    "include",
    "server",
    "route",
    "upstream",
    "fcgi_pool",
    "cache_policy",
    "modules",
    "static",
    "full_page_cache",
    "http3",
    "waf",
    "logging",
];

pub const SERVER_FIELD_CATALOG: &[&str] =
    &["listen", "server_name", "routes", "tls", "http3_listen"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_stable_and_parseable() {
        for code in [
            DiagnosticCode::UnsupportedConfigVersion,
            DiagnosticCode::UnknownField,
            DiagnosticCode::IncludeCycle,
            DiagnosticCode::DeprecatedField,
        ] {
            assert_eq!(DiagnosticCode::parse_stable(code.as_str()), Some(code));
        }
    }

    #[test]
    fn json_roundtrip_ordered() {
        let doc = DiagnosticDocument::new(vec![
            Diagnostic::error(DiagnosticCode::InvalidValue, "b").with_source("b.toml"),
            Diagnostic::error(DiagnosticCode::UnknownField, "a").with_source("a.toml"),
        ]);
        assert_eq!(doc.diagnostics[0].source.as_deref(), Some("a.toml"));
        let json = doc.to_json_string().unwrap();
        assert!(json.contains("\"schema_version\": 1"));
        assert!(!json.contains("password=secret"));
        let back: DiagnosticDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(back.schema_version, 1);
    }

    #[test]
    fn redact_password_and_redis() {
        assert!(redact_secrets("redis://:hunter2@127.0.0.1:6379/").contains("[REDACTED]"));
        assert!(redact_secrets("password=hunter2 ok").contains("[REDACTED]"));
    }

    #[test]
    fn typo_suggestion_bounded() {
        let s = suggest_typo("max_conection", &["max_connections", "timeout_ms"]).unwrap();
        assert!(s.contains("max_connections"));
        assert!(suggest_typo("zzzz", ROOT_FIELD_CATALOG).is_none());
    }

    #[test]
    fn human_render_deterministic() {
        let text =
            render_human(&[
                Diagnostic::error(DiagnosticCode::UnknownField, "unknown field `x`")
                    .with_source("c.toml")
                    .with_field("x")
                    .with_suggestion("remove `x`"),
            ]);
        assert!(text.contains("error[EXY-CONFIG-0002]"));
        assert!(text.contains("--> c.toml"));
    }
}
