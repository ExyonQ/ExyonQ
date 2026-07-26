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
use crate::diagnostic::{
    suggest_typo, Diagnostic, DiagnosticCode, Severity, SpanQuality, SourceSpan, ROOT_FIELD_CATALOG,
    SERVER_FIELD_CATALOG, span_from_byte_range,
};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("{0}")]
    Parse(String),

    /// TOML deserialize failure with optional byte-range span from the provider.
    #[error("{message}")]
    ParseLocated {
        message: String,
        source_text: String,
        byte_range: Option<std::ops::Range<usize>>,
    },

    #[error("unsupported config_version {found}; expected {expected}")]
    UnsupportedVersion { found: u32, expected: u32 },

    #[error("config must define at least one [[server]] block")]
    MissingServers,

    #[error("duplicate {kind} name: {name}")]
    DuplicateName { kind: &'static str, name: String },

    #[error("server references unknown route: {route}")]
    UnknownRoute { route: String },

    #[error("route `{route}` references unknown upstream: {upstream}")]
    UnknownUpstream { route: String, upstream: String },

    #[error("route `{route}` must define upstream, root, redirect, rewrite, or fastcgi")]
    RouteWithoutAction { route: String },

    #[error("route `{route}` defines conflicting actions (only one of upstream, root, redirect, rewrite, fastcgi)")]
    RouteConflictingActions { route: String },

    #[error("route `{route}` references unknown fcgi pool: {pool}")]
    UnknownFcgiPool { route: String, pool: String },

    #[error("fcgi pool `{pool}` address must not be empty")]
    EmptyFcgiPoolAddress { pool: String },

    #[error("fcgi pool `{pool}` max_concurrency must be 1..=4096, got {value}")]
    InvalidFcgiMaxConcurrency { pool: String, value: u32 },

    #[error(
        "fcgi pool `{pool}` max_connections must be 1..=max_concurrency ({max_concurrency}), got {value}"
    )]
    InvalidFcgiMaxConnections {
        pool: String,
        value: u32,
        max_concurrency: u32,
    },

    #[error("fcgi pool `{pool}` idle_timeout_ms must be 1..=3600000, got {value}")]
    InvalidFcgiIdleTimeout { pool: String, value: u64 },

    #[error("fcgi pool `{pool}` invalid transport: {detail}")]
    InvalidFcgiTransport { pool: String, detail: String },

    #[error("fcgi pool `{pool}` total_timeout_ms must be 1..=3600000, got {value}")]
    InvalidFcgiTotalTimeout { pool: String, value: u64 },

    #[error("fcgi pool `{pool}` checkout_timeout_ms must be 1..=600000, got {value}")]
    InvalidFcgiCheckoutTimeout { pool: String, value: u64 },

    #[error("route `{route}` with htaccess=overlay requires a document root (route `root` or fcgi_pool `document_root`)")]
    HtaccessRequiresDocumentRoot { route: String },

    #[error("route `{route}` with htaccess=overlay cannot combine with redirect")]
    HtaccessConflictsWithRedirect { route: String },

    #[error("route `{route}` with htaccess=overlay cannot combine with upstream proxy")]
    HtaccessConflictsWithUpstream { route: String },

    #[error("route `{route}` with htaccess=overlay requires fcgi_pool `{pool}` document_root")]
    HtaccessFcgiPoolMissingDocumentRoot { route: String, pool: String },

    #[error("invalid listen address: {value}")]
    InvalidListen { value: String },

    #[error("route match path must start with '/': {value}")]
    InvalidMatchPath { value: String },

    #[error("invalid upstream target (http only in v1): {value}")]
    InvalidUpstreamTarget { value: String },

    #[error("include file not found: {path}")]
    IncludeNotFound { path: PathBuf },

    #[error("include cycle detected: {path}")]
    IncludeCycle { path: PathBuf },

    #[error("include merge conflict: {message}")]
    IncludeConflict { message: String },

    #[error("invalid redirect status: {value}")]
    InvalidRedirectStatus { value: u16 },

    #[error("invalid rewrite target: {value}")]
    InvalidRewriteTarget { value: String },

    #[error("route `{route}` references unknown cache_policy: {policy}")]
    UnknownCachePolicy { route: String, policy: String },

    #[error("route `{route}` cache requires a static `root` or proxy `upstream` route")]
    CacheRequiresStaticOrProxyRoute { route: String },

    #[error("route `{route}` cache cannot combine with redirect, rewrite, or fastcgi")]
    CacheInvalidRouteAction { route: String },

    #[error("cache_policy `{policy}` ttl_seconds must be > 0")]
    InvalidCacheTtl { policy: String },

    #[error("cache_policy `{policy}` max_object_bytes must be > 0")]
    InvalidCacheMaxObject { policy: String },

    #[error("invalid static.preload: {message}")]
    InvalidStaticPreload { message: String },

    #[error("invalid http3.provider: {value} (allowed: s2n, quiche)")]
    InvalidHttp3Provider { value: String },

    #[error("invalid http3.max_ack_delay_ms: {value} (allowed: 0..=25000)")]
    InvalidHttp3MaxAckDelay { value: u64 },

    #[error("invalid http3.request_body_drain_cap_bytes: must be > 0")]
    InvalidHttp3DrainCap,
}

impl ConfigError {
    pub fn parse(err: toml::de::Error) -> Self {
        Self::Parse(err.to_string())
    }

    pub fn parse_toml(source_text: &str, err: toml::de::Error) -> Self {
        let message = err.to_string();
        let byte_range = err.span();
        Self::ParseLocated {
            message,
            source_text: source_text.to_string(),
            byte_range,
        }
    }

    pub fn with_path(self, path: &std::path::Path) -> Self {
        match self {
            Self::Parse(message) => Self::Parse(format!(
                "failed to parse config {}: {message}",
                path.display()
            )),
            Self::ParseLocated {
                message,
                source_text,
                byte_range,
            } => Self::ParseLocated {
                message: format!(
                    "failed to parse config {}: {message}",
                    path.display()
                ),
                source_text,
                byte_range,
            },
            other => other,
        }
    }

    /// Map this error into one structured diagnostic (P1.4-WS2).
    pub fn to_diagnostic(&self) -> Diagnostic {
        match self {
            Self::UnsupportedVersion { found, expected } => Diagnostic::error(
                DiagnosticCode::UnsupportedConfigVersion,
                format!("unsupported config_version {found}; expected {expected}"),
            )
            .with_field("config_version")
            .with_received(found.to_string())
            .with_expected(format!("1 or 2 (latest={expected})"))
            .with_documentation("docs/config/diagnostic-codes.md#exy-config-0001"),

            Self::ParseLocated {
                message,
                source_text,
                byte_range,
            } => {
                let mut d = Diagnostic::error(DiagnosticCode::UnknownField, message.clone());
                if message.contains("unknown field") {
                    d.code = DiagnosticCode::UnknownField;
                    if let Some(field) = extract_unknown_field(message) {
                        d.field = Some(field.clone());
                        d.received = Some(field.clone());
                        if let Some(sug) =
                            suggest_typo(&field, ROOT_FIELD_CATALOG).or_else(|| {
                                suggest_typo(&field, SERVER_FIELD_CATALOG)
                            })
                        {
                            d.suggestion = Some(sug);
                        }
                    }
                } else if message.contains("missing field") {
                    d.code = DiagnosticCode::MissingRequiredField;
                    d.severity = Severity::Error;
                } else {
                    d.code = DiagnosticCode::IrValidationError;
                }
                if let Some(range) = byte_range.clone() {
                    d.span = Some(span_from_byte_range(source_text, range));
                } else {
                    d.span = Some(SourceSpan {
                        start: crate::diagnostic::Position { line: 0, column: 0 },
                        end: crate::diagnostic::Position { line: 0, column: 0 },
                        quality: SpanQuality::SourceOnly,
                    });
                    // Prefer no fake 0,0 — clear span when source-only
                    d.span = None;
                }
                d
            }

            Self::Parse(message) => {
                let mut d = Diagnostic::error(DiagnosticCode::IrValidationError, message.clone());
                if message.contains("unknown field") {
                    d.code = DiagnosticCode::UnknownField;
                    if let Some(field) = extract_unknown_field(message) {
                        d.field = Some(field.clone());
                        if let Some(sug) = suggest_typo(&field, ROOT_FIELD_CATALOG) {
                            d.suggestion = Some(sug);
                        }
                    }
                }
                d
            }

            Self::MissingServers => Diagnostic::error(
                DiagnosticCode::MissingRequiredField,
                "config must define at least one [[server]] block",
            )
            .with_field("server"),

            Self::DuplicateName { kind, name } => Diagnostic::error(
                DiagnosticCode::DuplicateDefinition,
                format!("duplicate {kind} name: {name}"),
            )
            .with_field(*kind)
            .with_received(name.clone()),

            Self::IncludeNotFound { path } => Diagnostic::error(
                DiagnosticCode::IncludeNotFound,
                format!("include file not found: {}", path.display()),
            )
            .with_received(path.display().to_string()),

            Self::IncludeCycle { path } => Diagnostic::error(
                DiagnosticCode::IncludeCycle,
                format!("include cycle detected: {}", path.display()),
            )
            .with_received(path.display().to_string()),

            Self::IncludeConflict { message } => Diagnostic::error(
                DiagnosticCode::MergeConflict,
                format!("include merge conflict: {message}"),
            ),

            Self::InvalidListen { value }
            | Self::InvalidMatchPath { value }
            | Self::InvalidUpstreamTarget { value }
            | Self::InvalidRewriteTarget { value } => Diagnostic::error(
                DiagnosticCode::InvalidValue,
                self.to_string(),
            )
            .with_received(value.clone()),

            Self::InvalidRedirectStatus { value } => Diagnostic::error(
                DiagnosticCode::InvalidValue,
                self.to_string(),
            )
            .with_received(value.to_string()),

            other => Diagnostic::error(DiagnosticCode::IrValidationError, other.to_string()),
        }
    }
}

fn extract_unknown_field(message: &str) -> Option<String> {
    // serde: unknown field `foo`, expected one of ...
    let start = message.find('`')? + 1;
    let end = message[start..].find('`')? + start;
    Some(message[start..end].to_string())
}

#[cfg(test)]
impl PartialEq for ConfigError {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}
