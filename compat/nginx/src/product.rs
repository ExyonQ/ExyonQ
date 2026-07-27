//! P1.4-WS6 product taxonomy + operator report envelope for NGINX import.
//!
//! Internal `CompatStatus` remains the importer engine. This module maps to the
//! frozen product taxonomy without a second IR.

use crate::report::{CompatStatus, CompatibilityReport, MigrationSummary};
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Frozen product classification (P1.4-WS6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImportTaxonomy {
    Exact,
    SupportedWithNormalization,
    SupportedWithDocumentedLimit,
    Lossy,
    Unsupported,
    IgnoredSafe,
    RejectedUnsafe,
    InvalidSource,
}

impl ImportTaxonomy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "EXACT",
            Self::SupportedWithNormalization => "SUPPORTED_WITH_NORMALIZATION",
            Self::SupportedWithDocumentedLimit => "SUPPORTED_WITH_DOCUMENTED_LIMIT",
            Self::Lossy => "LOSSY",
            Self::Unsupported => "UNSUPPORTED",
            Self::IgnoredSafe => "IGNORED_SAFE",
            Self::RejectedUnsafe => "REJECTED_UNSAFE",
            Self::InvalidSource => "INVALID_SOURCE",
        }
    }

    pub fn diagnostic_code(self) -> &'static str {
        match self {
            Self::Exact | Self::SupportedWithNormalization | Self::SupportedWithDocumentedLimit => {
                ""
            }
            Self::Lossy => "EXY-IMPORT-0003",
            Self::Unsupported => "EXY-IMPORT-0002",
            Self::IgnoredSafe => "",
            Self::RejectedUnsafe => "EXY-IMPORT-0004",
            Self::InvalidSource => "EXY-IMPORT-0001",
        }
    }
}

/// Map engine status (+ message heuristics) → product taxonomy.
pub fn taxonomy_of(status: CompatStatus, message: &str) -> ImportTaxonomy {
    let lower = message.to_ascii_lowercase();
    match status {
        CompatStatus::Supported => {
            if lower.contains("normaliz") {
                ImportTaxonomy::SupportedWithNormalization
            } else if lower.contains("limit") || lower.contains("partial sni") {
                ImportTaxonomy::SupportedWithDocumentedLimit
            } else {
                ImportTaxonomy::Exact
            }
        }
        CompatStatus::Partial => {
            if lower.contains("limit") || lower.contains("documented") || lower.contains("sni") {
                ImportTaxonomy::SupportedWithDocumentedLimit
            } else {
                ImportTaxonomy::Lossy
            }
        }
        CompatStatus::Unsupported => ImportTaxonomy::Unsupported,
        CompatStatus::Ignored => ImportTaxonomy::IgnoredSafe,
        CompatStatus::Error => {
            if lower.contains("unsafe")
                || lower.contains("ssrf")
                || lower.contains("credential")
                || lower.contains("password")
                || lower.contains("traversal")
            {
                ImportTaxonomy::RejectedUnsafe
            } else if lower.contains("context") {
                // Context mismatch still uses IMPORT-0005 via caller when known;
                // default invalid/parse path.
                ImportTaxonomy::InvalidSource
            } else {
                ImportTaxonomy::InvalidSource
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProductCompatEntry {
    pub taxonomy: ImportTaxonomy,
    pub diagnostic_code: String,
    pub file: String,
    pub line: u32,
    pub directive: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mapping: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended_action: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TaxonomyCounts {
    pub exact: usize,
    pub normalized: usize,
    pub limit: usize,
    pub lossy: usize,
    pub unsupported: usize,
    pub ignored_safe: usize,
    pub rejected: usize,
    pub invalid_source: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProductImportReport {
    pub schema_version: u32,
    pub importer: &'static str,
    pub importer_version: &'static str,
    pub source_file: String,
    pub source_hash: String,
    pub target_ir_version: u32,
    pub taxonomy: TaxonomyCounts,
    pub warnings: usize,
    pub errors: usize,
    pub output_fingerprint: String,
    pub runtimeplan_compile_result: String,
    pub total_compat_promise: &'static str,
    pub entries: Vec<ProductCompatEntry>,
    pub engine_summary: MigrationSummary,
}

pub const IMPORTER_VERSION: &str = "1.0.0-ws6";
pub const TARGET_IR_VERSION: u32 = 2;

fn fingerprint_str(s: &str) -> String {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

impl ProductImportReport {
    pub fn from_migrate(
        source_file: &str,
        source_bytes: &[u8],
        config_toml: &str,
        report: &CompatibilityReport,
        runtimeplan_compile_result: &str,
    ) -> Self {
        let mut taxonomy = TaxonomyCounts::default();
        let mut entries = Vec::with_capacity(report.entries.len());
        let mut warnings = 0usize;
        let mut errors = 0usize;
        for e in &report.entries {
            let tax = taxonomy_of(e.status, &e.message);
            match tax {
                ImportTaxonomy::Exact => taxonomy.exact += 1,
                ImportTaxonomy::SupportedWithNormalization => taxonomy.normalized += 1,
                ImportTaxonomy::SupportedWithDocumentedLimit => {
                    taxonomy.limit += 1;
                    warnings += 1;
                }
                ImportTaxonomy::Lossy => {
                    taxonomy.lossy += 1;
                    warnings += 1;
                }
                ImportTaxonomy::Unsupported => {
                    taxonomy.unsupported += 1;
                    warnings += 1;
                }
                ImportTaxonomy::IgnoredSafe => taxonomy.ignored_safe += 1,
                ImportTaxonomy::RejectedUnsafe => {
                    taxonomy.rejected += 1;
                    errors += 1;
                }
                ImportTaxonomy::InvalidSource => {
                    taxonomy.invalid_source += 1;
                    errors += 1;
                }
            }
            let code = tax.diagnostic_code();
            entries.push(ProductCompatEntry {
                taxonomy: tax,
                diagnostic_code: if code.is_empty() {
                    String::new()
                } else {
                    code.to_string()
                },
                file: e.file.clone(),
                line: e.line,
                directive: e.directive.clone(),
                message: e.message.clone(),
                mapping: e.mapping.clone(),
                recommended_action: e.migration_action.clone().or_else(|| match tax {
                    ImportTaxonomy::Unsupported => {
                        Some("rewrite manually in AppConfig TOML".into())
                    }
                    ImportTaxonomy::Lossy => Some("review IR; enable --strict to fail CI".into()),
                    ImportTaxonomy::RejectedUnsafe => {
                        Some("remove unsafe directive before migrate".into())
                    }
                    ImportTaxonomy::InvalidSource => Some("fix source syntax".into()),
                    _ => None,
                }),
            });
        }
        Self {
            schema_version: 1,
            importer: "nginx",
            importer_version: IMPORTER_VERSION,
            source_file: source_file.to_string(),
            source_hash: fingerprint_str(&String::from_utf8_lossy(source_bytes)),
            target_ir_version: TARGET_IR_VERSION,
            taxonomy,
            warnings,
            errors,
            output_fingerprint: fingerprint_str(config_toml),
            runtimeplan_compile_result: runtimeplan_compile_result.to_string(),
            total_compat_promise: "FORBIDDEN",
            entries,
            engine_summary: report.summary.clone(),
        }
    }

    pub fn render_human(&self) -> String {
        let mut out = String::new();
        out.push_str("# NGINX → ExyonQ import report (P1.4-WS6)\n\n");
        out.push_str(&format!("SOURCE_FILE = {}\n", self.source_file));
        out.push_str(&format!("SOURCE_HASH = {}\n", self.source_hash));
        out.push_str(&format!("IMPORTER_VERSION = {}\n", self.importer_version));
        out.push_str(&format!("TARGET_IR_VERSION = {}\n", self.target_ir_version));
        out.push_str(&format!("EXACT_COUNT = {}\n", self.taxonomy.exact));
        out.push_str(&format!(
            "NORMALIZED_COUNT = {}\n",
            self.taxonomy.normalized
        ));
        out.push_str(&format!("LIMIT_COUNT = {}\n", self.taxonomy.limit));
        out.push_str(&format!("LOSSY_COUNT = {}\n", self.taxonomy.lossy));
        out.push_str(&format!(
            "UNSUPPORTED_COUNT = {}\n",
            self.taxonomy.unsupported
        ));
        out.push_str(&format!("REJECTED_COUNT = {}\n", self.taxonomy.rejected));
        out.push_str(&format!(
            "INVALID_SOURCE_COUNT = {}\n",
            self.taxonomy.invalid_source
        ));
        out.push_str(&format!("WARNINGS = {}\n", self.warnings));
        out.push_str(&format!("ERRORS = {}\n", self.errors));
        out.push_str(&format!(
            "OUTPUT_FINGERPRINT = {}\n",
            self.output_fingerprint
        ));
        out.push_str(&format!(
            "RUNTIMEPLAN_COMPILE_RESULT = {}\n",
            self.runtimeplan_compile_result
        ));
        out.push_str("TOTAL_COMPAT_PROMISE = FORBIDDEN\n\n");
        out.push_str("## Entries\n\n");
        if self.entries.is_empty() {
            out.push_str("- none\n");
        } else {
            for e in &self.entries {
                out.push_str(&format!(
                    "- [{}] {}:{} `{}` — {}",
                    e.taxonomy.as_str(),
                    e.file,
                    e.line,
                    e.directive,
                    e.message
                ));
                if !e.diagnostic_code.is_empty() {
                    out.push_str(&format!(" ({})", e.diagnostic_code));
                }
                out.push('\n');
                if let Some(a) = &e.recommended_action {
                    out.push_str(&format!("  action: {a}\n"));
                }
            }
        }
        out
    }

    pub fn render_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| e.to_string())
    }

    /// Exit policy: errors/rejected/invalid/unsupported always fail;
    /// `--strict` also fails on LOSSY and documented-limit classes that are lossy-adjacent.
    pub fn exit_code(&self, strict: bool) -> i32 {
        if self.taxonomy.rejected > 0
            || self.taxonomy.invalid_source > 0
            || self.taxonomy.unsupported > 0
            || self.errors > 0
        {
            return 1;
        }
        if strict && (self.taxonomy.lossy > 0 || self.taxonomy.limit > 0) {
            return 1;
        }
        0
    }
}

/// Redact product report messages for operator emit.
pub fn redact_product_report(mut report: ProductImportReport) -> ProductImportReport {
    use exyonq_config_ir::redact_secrets;
    report.source_file = redact_secrets(&report.source_file);
    for e in &mut report.entries {
        e.message = redact_secrets(&e.message);
        if let Some(m) = e.mapping.take() {
            e.mapping = Some(redact_secrets(&m));
        }
        if let Some(a) = e.recommended_action.take() {
            e.recommended_action = Some(redact_secrets(&a));
        }
        e.file = redact_secrets(&e.file);
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taxonomy_map_stable() {
        assert_eq!(
            taxonomy_of(CompatStatus::Supported, "ok"),
            ImportTaxonomy::Exact
        );
        assert_eq!(
            taxonomy_of(CompatStatus::Partial, "lossy rewrite"),
            ImportTaxonomy::Lossy
        );
        assert_eq!(
            taxonomy_of(CompatStatus::Unsupported, "regex"),
            ImportTaxonomy::Unsupported
        );
        assert_eq!(
            taxonomy_of(CompatStatus::Error, "unsafe ssrf target"),
            ImportTaxonomy::RejectedUnsafe
        );
    }

    #[test]
    fn fingerprint_deterministic() {
        let a = fingerprint_str("abc");
        let b = fingerprint_str("abc");
        assert_eq!(a, b);
        assert_ne!(fingerprint_str("abc"), fingerprint_str("abd"));
    }
}
