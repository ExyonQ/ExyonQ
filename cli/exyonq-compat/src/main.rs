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
use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use exyonq_compat_common::MigrationReport;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "exyonq-compat",
    version = env!("CARGO_PKG_VERSION"),
    about = "Offline compatibility migration for ExyonQ"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Migrate legacy config into native ExyonQ TOML + report.
    Migrate {
        #[arg(long, value_enum)]
        from: SourceFormat,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        report: PathBuf,
    },
}

#[derive(Debug, Clone, ValueEnum)]
enum SourceFormat {
    Nginx,
    Apache,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Commands::Migrate {
            from,
            input,
            out,
            report,
        } => match migrate_command(from, &input, &out, &report) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err:#}");
                ExitCode::from(1)
            }
        },
    }
}

fn migrate_command(
    from: SourceFormat,
    input: &Path,
    out: &Path,
    report_path: &Path,
) -> anyhow::Result<()> {
    let (toml_result, migration_report): (anyhow::Result<String>, MigrationReport) = match from {
        // AP-07: path-based Full migrate expands `include` from disk.
        SourceFormat::Nginx => {
            match exyonq_compat_nginx::migrate_file(input, &exyonq_compat_nginx::MigrateOptions::default())
            {
                Ok(out) => {
                    let legacy = out.report.to_legacy(exyonq_compat_nginx::IMPORTER_NAME);
                    (
                        if out.config.trim().is_empty() && out.report.has_errors() {
                            Err(anyhow::anyhow!("nginx migration refused"))
                        } else {
                            Ok(out.config)
                        },
                        MigrationReport {
                            source: legacy.source,
                            notes: legacy.notes,
                            unsupported: legacy.unsupported,
                        },
                    )
                }
                Err(e) => (
                    Err(e),
                    MigrationReport {
                        source: "nginx".into(),
                        notes: vec![],
                        unsupported: vec![],
                    },
                ),
            }
        }
        // AP-07 / ADR-053: expand Include / IncludeOptional from the entry path.
        SourceFormat::Apache => exyonq_compat_apache::migrate_path_with_report(input),
    };

    let report_output = if report_path.extension().and_then(|ext| ext.to_str()) == Some("json") {
        migration_report.render_json()?
    } else {
        migration_report.render_markdown()
    };
    std::fs::write(report_path, report_output)
        .with_context(|| format!("failed writing report {}", report_path.display()))?;

    let toml_output = toml_result.with_context(|| {
        format!(
            "migration refused (report written to {})",
            report_path.display()
        )
    })?;

    std::fs::write(out, toml_output)
        .with_context(|| format!("failed writing output {}", out.display()))?;

    println!(
        "migrated {} -> {} (report: {})",
        input.display(),
        out.display(),
        report_path.display()
    );
    Ok(())
}
