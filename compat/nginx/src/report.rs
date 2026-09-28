//! Compatibility report for NGINX → ExyonQ import.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompatStatus {
    Supported,
    Partial,
    Unsupported,
    Ignored,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLoc {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

impl SourceLoc {
    /// Fallback location when the lexer did not attach a precise span.
    pub fn unlocated(file: &str) -> Self {
        Self {
            file: file.to_string(),
            line: 1,
            column: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatEntry {
    pub file: String,
    pub line: u32,
    pub directive: String,
    pub status: CompatStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mapping: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub migration_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_form: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variables: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationSummary {
    pub servers_parsed: usize,
    pub listeners_generated: usize,
    pub sites_generated: usize,
    pub routes_generated: usize,
    pub upstreams_generated: usize,
    pub fcgi_pools_generated: usize,
    pub nginx_upstream_blocks_parsed: usize,
    pub http_upstreams_generated: usize,
    pub fcgi_pools_from_upstreams: usize,
    pub unresolved_upstream_references: usize,
    pub inherited_roots: usize,
    pub inherited_indexes: usize,
    pub exact_locations: usize,
    pub prefix_locations: usize,
    pub preferential_prefixes: usize,
    pub regex_locations: usize,
    pub named_locations: usize,
    pub try_files_directives: usize,
    pub rewrite_rules: usize,
    pub safe_redirect_rewrites: usize,
    pub internal_rewrites_unsupported: usize,
    pub location_precedence_risks: usize,
    pub unresolved_named_locations: usize,
    pub supported_directives: usize,
    pub partial_directives: usize,
    pub unsupported_directives: usize,
    pub ignored_directives: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, Default)]
pub struct CompatibilityReport {
    pub entries: Vec<CompatEntry>,
    pub summary: MigrationSummary,
}

impl CompatibilityReport {
    pub fn push(
        &mut self,
        loc: &SourceLoc,
        directive: &str,
        status: CompatStatus,
        message: impl Into<String>,
        mapping: Option<String>,
    ) {
        let entry = CompatEntry {
            file: loc.file.clone(),
            line: loc.line,
            directive: directive.to_string(),
            status,
            message: message.into(),
            mapping,
            migration_action: None,
            location_kind: None,
            semantic_form: None,
            variables: None,
        };
        match status {
            CompatStatus::Supported => self.summary.supported_directives += 1,
            CompatStatus::Partial => self.summary.partial_directives += 1,
            CompatStatus::Unsupported => self.summary.unsupported_directives += 1,
            CompatStatus::Ignored => self.summary.ignored_directives += 1,
            CompatStatus::Error => self.summary.errors += 1,
        }
        self.entries.push(entry);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_migration(
        &mut self,
        loc: &SourceLoc,
        directive: &str,
        status: CompatStatus,
        message: impl Into<String>,
        mapping: Option<String>,
        migration_action: Option<String>,
        location_kind: Option<&str>,
        semantic_form: Option<String>,
        variables: Option<Vec<String>>,
    ) {
        let entry = CompatEntry {
            file: loc.file.clone(),
            line: loc.line,
            directive: directive.to_string(),
            status,
            message: message.into(),
            mapping,
            migration_action,
            location_kind: location_kind.map(str::to_string),
            semantic_form,
            variables,
        };
        match status {
            CompatStatus::Supported => self.summary.supported_directives += 1,
            CompatStatus::Partial => self.summary.partial_directives += 1,
            CompatStatus::Unsupported => self.summary.unsupported_directives += 1,
            CompatStatus::Ignored => self.summary.ignored_directives += 1,
            CompatStatus::Error => self.summary.errors += 1,
        }
        self.entries.push(entry);
    }

    pub fn has_errors(&self) -> bool {
        self.summary.errors > 0
    }

    pub fn has_partial_or_unsupported(&self) -> bool {
        self.summary.partial_directives > 0 || self.summary.unsupported_directives > 0
    }

    pub fn render_text(&self) -> String {
        let mut out = String::new();
        out.push_str("# NGINX → ExyonQ compatibility report\n\n");
        out.push_str("## Summary\n\n");
        let s = &self.summary;
        out.push_str(&format!("- servers parsed: {}\n", s.servers_parsed));
        out.push_str(&format!(
            "- listeners generated: {}\n",
            s.listeners_generated
        ));
        out.push_str(&format!("- sites generated: {}\n", s.sites_generated));
        out.push_str(&format!("- routes generated: {}\n", s.routes_generated));
        out.push_str(&format!(
            "- upstreams generated: {}\n",
            s.upstreams_generated
        ));
        out.push_str(&format!(
            "- fcgi pools generated: {}\n",
            s.fcgi_pools_generated
        ));
        out.push_str(&format!(
            "- nginx upstream blocks parsed: {}\n",
            s.nginx_upstream_blocks_parsed
        ));
        out.push_str(&format!(
            "- http upstreams generated: {}\n",
            s.http_upstreams_generated
        ));
        out.push_str(&format!(
            "- fcgi pools from upstreams: {}\n",
            s.fcgi_pools_from_upstreams
        ));
        out.push_str(&format!(
            "- unresolved upstream references: {}\n",
            s.unresolved_upstream_references
        ));
        out.push_str(&format!("- inherited roots: {}\n", s.inherited_roots));
        out.push_str(&format!("- inherited indexes: {}\n", s.inherited_indexes));
        out.push_str(&format!("- exact locations: {}\n", s.exact_locations));
        out.push_str(&format!("- prefix locations: {}\n", s.prefix_locations));
        out.push_str(&format!(
            "- preferential prefixes: {}\n",
            s.preferential_prefixes
        ));
        out.push_str(&format!("- regex locations: {}\n", s.regex_locations));
        out.push_str(&format!("- named locations: {}\n", s.named_locations));
        out.push_str(&format!(
            "- try_files directives: {}\n",
            s.try_files_directives
        ));
        out.push_str(&format!("- rewrite rules: {}\n", s.rewrite_rules));
        out.push_str(&format!(
            "- safe redirect rewrites: {}\n",
            s.safe_redirect_rewrites
        ));
        out.push_str(&format!(
            "- internal rewrites unsupported: {}\n",
            s.internal_rewrites_unsupported
        ));
        out.push_str(&format!(
            "- location precedence risks: {}\n",
            s.location_precedence_risks
        ));
        out.push_str(&format!(
            "- unresolved named locations: {}\n",
            s.unresolved_named_locations
        ));
        out.push_str(&format!(
            "- supported directives: {}\n",
            s.supported_directives
        ));
        out.push_str(&format!("- partial directives: {}\n", s.partial_directives));
        out.push_str(&format!(
            "- unsupported directives: {}\n",
            s.unsupported_directives
        ));
        out.push_str(&format!("- ignored directives: {}\n", s.ignored_directives));
        out.push_str(&format!("- errors: {}\n\n", s.errors));
        out.push_str("## Entries\n\n");
        if self.entries.is_empty() {
            out.push_str("- none\n");
        } else {
            for e in &self.entries {
                out.push_str(&format!(
                    "- [{}] {}:{} `{}` — {} ({:?})\n",
                    status_label(e.status),
                    e.file,
                    e.line,
                    e.directive,
                    e.message,
                    e.mapping
                ));
                if let Some(action) = &e.migration_action {
                    out.push_str(&format!("  action: {action}\n"));
                }
                if let Some(form) = &e.semantic_form {
                    out.push_str(&format!("  semantic: {form}\n"));
                }
            }
        }
        out
    }

    pub fn render_json(&self) -> serde_json::Result<String> {
        #[derive(Serialize)]
        struct Out<'a> {
            summary: &'a MigrationSummary,
            entries: &'a [CompatEntry],
        }
        serde_json::to_string_pretty(&Out {
            summary: &self.summary,
            entries: &self.entries,
        })
    }
}

fn status_label(status: CompatStatus) -> &'static str {
    match status {
        CompatStatus::Supported => "SUPPORTED",
        CompatStatus::Partial => "PARTIAL",
        CompatStatus::Unsupported => "UNSUPPORTED",
        CompatStatus::Ignored => "IGNORED",
        CompatStatus::Error => "ERROR",
    }
}

/// Legacy report adapter for `exyonq-compat` and integration tests.
#[derive(Debug, Clone)]
pub struct LegacyMigrationReport {
    pub source: String,
    pub notes: Vec<exyonq_compat_common::CompatNote>,
    pub unsupported: Vec<String>,
}

impl CompatibilityReport {
    pub fn to_legacy(&self, source: &str) -> LegacyMigrationReport {
        let mut notes = Vec::new();
        let mut unsupported = Vec::new();
        for e in &self.entries {
            let level = match e.status {
                CompatStatus::Supported => exyonq_compat_common::CompatLevel::Migrated,
                CompatStatus::Partial => exyonq_compat_common::CompatLevel::Approximated,
                CompatStatus::Unsupported => exyonq_compat_common::CompatLevel::Unsupported,
                CompatStatus::Ignored => exyonq_compat_common::CompatLevel::Approximated,
                CompatStatus::Error => exyonq_compat_common::CompatLevel::Unsupported,
            };
            if matches!(e.status, CompatStatus::Unsupported | CompatStatus::Error) {
                unsupported.push(format!(
                    "{}:{} `{}`: {}",
                    e.file, e.line, e.directive, e.message
                ));
            } else {
                notes.push(exyonq_compat_common::CompatNote {
                    level,
                    message: format!("{}:{} `{}`: {}", e.file, e.line, e.directive, e.message),
                });
            }
        }
        LegacyMigrationReport {
            source: source.to_string(),
            notes,
            unsupported,
        }
    }
}
