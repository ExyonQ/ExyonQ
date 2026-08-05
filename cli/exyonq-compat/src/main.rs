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
use std::path::PathBuf;

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
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Migrate {
            from,
            input,
            out,
            report,
        } => migrate_command(from, &input, &out, &report)?,
    }

    Ok(())
}

fn migrate_command(
    from: SourceFormat,
    input: &PathBuf,
    out: &PathBuf,
    report: &PathBuf,
) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(input)
        .with_context(|| format!("failed reading input {}", input.display()))?;

    let (toml_output, migration_report) = match from {
        SourceFormat::Nginx => exyonq_compat_nginx::migrate(&source)?,
    };

    std::fs::write(out, toml_output)
        .with_context(|| format!("failed writing output {}", out.display()))?;

    let report_output = if report.extension().and_then(|ext| ext.to_str()) == Some("json") {
        migration_report.render_json()?
    } else {
        migration_report.render_markdown()
    };

    std::fs::write(report, report_output)
        .with_context(|| format!("failed writing report {}", report.display()))?;

    println!(
        "migrated {} -> {} (report: {})",
        input.display(),
        out.display(),
        report.display()
    );

    Ok(())
}
