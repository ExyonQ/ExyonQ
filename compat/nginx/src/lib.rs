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
//! NGINX → ExyonQ offline importer (Plan 09 Tier 1).

mod analyze;
mod ast;
mod include;
mod lexer;
mod limits;
mod location;
mod map_ir;
mod parser;
mod product;
mod report;
mod rewrite;
mod try_files;
mod upstream;
mod variables;

use anyhow::{Context, Result};
use exyonq_compat_common::MigrationReport;
use std::path::Path;

pub const IMPORTER_NAME: &str = "nginx";

pub use product::{
    redact_product_report, taxonomy_of, ImportTaxonomy, ProductImportReport, IMPORTER_VERSION,
    TARGET_IR_VERSION,
};
pub use report::{
    CompatEntry, CompatStatus, CompatStatus as NginxCompatStatus, CompatibilityReport,
    MigrationSummary,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    Toml,
    Json,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReportFormat {
    #[default]
    Text,
    Json,
}

#[derive(Debug, Clone, Default)]
pub struct MigrateOptions {
    pub dry_run: bool,
    pub strict: bool,
    pub format: OutputFormat,
    pub report_format: ReportFormat,
}

#[derive(Debug, Clone)]
pub struct MigrateOutput {
    pub config: String,
    pub report: CompatibilityReport,
    pub summary: MigrationSummary,
}

impl MigrateOutput {
    pub fn exit_code(&self, strict: bool) -> i32 {
        if self.report.has_errors() {
            return 1;
        }
        if strict && self.report.has_partial_or_unsupported() {
            return 1;
        }
        0
    }
}

/// Migrate NGINX config from a filesystem path (includes expanded).
pub fn migrate_file(path: &Path, options: &MigrateOptions) -> Result<MigrateOutput> {
    let mut report = CompatibilityReport::default();
    let tokens = include::load_config(path, &mut report)
        .with_context(|| format!("failed loading {}", path.display()))?;
    run_pipeline(tokens, &mut report, options, true)
}

/// Migrate NGINX config from in-memory source (includes not expanded from disk).
pub fn migrate_source(label: &str, input: &str, options: &MigrateOptions) -> Result<MigrateOutput> {
    let mut report = CompatibilityReport::default();
    let tokens = include::load_config_string(label, input, &mut report)?;
    run_pipeline(tokens, &mut report, options, false)
}

/// Legacy API used by `exyonq-compat` and integration tests.
pub fn migrate(input: &str) -> Result<(String, MigrationReport)> {
    let output = migrate_source("input.conf", input, &MigrateOptions::default())?;
    let legacy = output.report.to_legacy(IMPORTER_NAME);
    Ok((
        output.config,
        MigrationReport {
            source: legacy.source,
            notes: legacy.notes,
            unsupported: legacy.unsupported,
        },
    ))
}

fn run_pipeline(
    tokens: Vec<lexer::Token>,
    report: &mut CompatibilityReport,
    options: &MigrateOptions,
    expand_includes: bool,
) -> Result<MigrateOutput> {
    let config = include::parse_tokens(&tokens, report)?;
    if !expand_includes {
        note_unexpanded_includes(&config, report);
    }
    let analyzed = analyze::analyze(&config, report);
    let mapped = map_ir::map_to_ir(&analyzed, report);
    map_ir::validate_mapped(&mapped).context("generated IR failed validation")?;

    let summary = report.summary.clone();
    let config_out = match options.format {
        OutputFormat::Toml => mapped.toml.clone(),
        OutputFormat::Json => serde_json::json!({
            "config_toml": mapped.toml,
            "summary": summary,
        })
        .to_string(),
    };

    Ok(MigrateOutput {
        config: config_out,
        report: std::mem::take(report),
        summary,
    })
}

/// Handle bare `include` directives when loading from string (no filesystem expansion).
pub fn note_unexpanded_includes(config: &ast::Config, report: &mut CompatibilityReport) {
    for directive in &config.directives {
        note_include_recursive(directive, report);
    }
}

fn note_include_recursive(directive: &ast::Directive, report: &mut CompatibilityReport) {
    if directive.name == "include" {
        let pattern = directive.args.join(" ");
        report.push(
            &directive.loc,
            "include",
            CompatStatus::Partial,
            format!("include `{pattern}` not expanded without filesystem input"),
            None,
        );
    }
    if let Some(block) = &directive.block {
        for child in &block.directives {
            note_include_recursive(child, report);
        }
    }
}
