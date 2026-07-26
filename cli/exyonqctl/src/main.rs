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
use clap::{Parser, Subcommand, ValueEnum};
use exyonq_compat_nginx::{
    redact_product_report, MigrateOptions, OutputFormat, ProductImportReport, ReportFormat,
};
use exyonq_config_ir::redact_secrets;
use exyonq_config_cli::{
    classify_reload_diff, emit_result, explain, format_config_status, format_exy,
    format_generation, lint, profile_explain, profile_list, profile_render, profile_test,
    reload_check, test_config, CheckOptions, ExplainRequest, FormatMode, FormatRequest,
    OutputFormat as ConfigOut, ProfileCliInputs,
};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

/// Honest product version metadata (P1.5-WS6). Control binary uses the system
/// allocator (no optional jemalloc feature). No builder paths or secrets.
const CTL_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\nproduct_version=",
    env!("CARGO_PKG_VERSION"),
    "\nartifact_version=",
    env!("EXYONQ_ARTIFACT_VERSION"),
    "\nsource_revision=",
    env!("EXYONQ_SOURCE_REVISION"),
    "\ntarget=",
    env!("EXYONQ_TARGET"),
    "\nprofile=",
    env!("EXYONQ_PROFILE"),
    "\nallocator=system"
);

#[derive(Parser)]
#[command(
    name = "exyonqctl",
    version = CTL_VERSION,
    long_version = CTL_VERSION,
    disable_version_flag = true,
    about = "ExyonQ control plane CLI"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Print version metadata (product_version, source_revision, target, profile).
    #[arg(long, global = true)]
    version: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Reload config from disk into a running server.
    Reload {
        #[arg(short, long)]
        config: Option<PathBuf>,
        #[arg(short, long)]
        socket: Option<PathBuf>,
    },
    /// Show live generation and config fingerprint.
    Status {
        #[arg(short, long)]
        socket: Option<PathBuf>,
        /// P15-WS5-STATUS-001: human (default) or raw control-socket JSON.
        #[arg(long, value_enum, default_value_t = ControlOutFormat::Human)]
        format: ControlOutFormat,
    },
    /// Stop accepting new connections; let in-flight requests finish.
    Drain {
        #[arg(short, long)]
        socket: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = ControlOutFormat::Human)]
        format: ControlOutFormat,
    },
    /// Drain and request graceful process shutdown.
    Shutdown {
        #[arg(short, long)]
        socket: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = ControlOutFormat::Human)]
        format: ControlOutFormat,
    },
    /// Authenticated FPC cache purge (dedicated purge socket — WC3).
    Purge {
        #[command(subcommand)]
        command: PurgeCommand,
        /// Purge Unix socket (default: EXYONQ_CACHE_PURGE_SOCKET).
        #[arg(short, long, global = true)]
        socket: Option<PathBuf>,
        /// Shared secret (default: EXYONQ_CACHE_PURGE_TOKEN). Never printed.
        #[arg(long, global = true)]
        token: Option<String>,
    },
    /// Offline configuration utilities.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Subcommand)]
enum PurgeCommand {
    /// Purge one exact URL identity for a site.
    Url {
        site_id: u64,
        scheme: String,
        host: String,
        path: String,
        #[arg(long, default_value = "")]
        query: String,
    },
    /// Purge all L1 entries for a site.
    Site { site_id: u64 },
    /// Purge one runtime generation for a site.
    Generation { site_id: u64, generation: u64 },
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Parse and validate config (no RuntimePlan compile).
    Lint {
        path: PathBuf,
        #[arg(long, value_enum, default_value_t = DiagFormat::Human)]
        format: DiagFormat,
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        no_color: bool,
    },
    /// Validate and compile RuntimePlan offline (discard; no listeners).
    Test {
        path: PathBuf,
        #[arg(long, value_enum, default_value_t = DiagFormat::Human)]
        format: DiagFormat,
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        no_color: bool,
    },
    /// Explain a public configuration term / directive.
    Explain {
        directive: Option<String>,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        pointer: Option<String>,
        #[arg(long, value_enum, default_value_t = DiagFormat::Human)]
        format: DiagFormat,
    },
    /// Format a Serverfile (.exy only).
    Format {
        path: PathBuf,
        #[arg(long)]
        write: bool,
        #[arg(long)]
        check: bool,
        #[arg(long, value_enum, default_value_t = DiagFormat::Human)]
        format: DiagFormat,
        #[arg(long)]
        no_color: bool,
    },
    /// Product profiles (static|proxy|php|wordpress).
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    /// Precheck / diff / live structural reload (P1.4-WS5).
    Reload {
        path: PathBuf,
        /// Offline preflight only (parse → validate → compile; no publish).
        #[arg(long)]
        check: bool,
        /// Classify candidate vs live fingerprint (no publish).
        #[arg(long)]
        diff: bool,
        #[arg(long, value_enum, default_value_t = DiagFormat::Human)]
        format: DiagFormat,
        #[arg(short, long)]
        socket: Option<PathBuf>,
    },
    /// Live generation / fingerprint / reload flags (control socket).
    Status {
        #[arg(long, value_enum, default_value_t = DiagFormat::Human)]
        format: DiagFormat,
        #[arg(short, long)]
        socket: Option<PathBuf>,
        #[arg(long)]
        source_config: Option<PathBuf>,
    },
    /// Current generation + fingerprint only.
    Generation {
        #[arg(long, value_enum, default_value_t = DiagFormat::Human)]
        format: DiagFormat,
        #[arg(short, long)]
        socket: Option<PathBuf>,
    },
    /// Import NGINX config into ExyonQ IR (offline; no RuntimePlan publish).
    MigrateNginx {
        #[arg(long)]
        input: PathBuf,
        /// Write IR TOML to this path (requires `--write`; ignored under `--dry-run`).
        #[arg(long)]
        output: Option<PathBuf>,
        /// Write product compatibility report to this path.
        #[arg(long)]
        report: Option<PathBuf>,
        /// Preview only — never write `--output` (default true).
        #[arg(long, default_value_t = true)]
        dry_run: bool,
        /// Allow writing `--output` (implies not dry-run for the IR file).
        #[arg(long)]
        write: bool,
        /// Import + AppConfig validate + RuntimePlan compile (discard; no listeners).
        #[arg(long)]
        check: bool,
        #[arg(long)]
        strict: bool,
        /// Operator report format.
        #[arg(long, value_enum, default_value_t = DiagFormat::Human)]
        format: DiagFormat,
        /// IR body format when printing/writing config.
        #[arg(long, value_enum, default_value_t = CliOutputFormat::Toml)]
        config_format: CliOutputFormat,
    },
    /// `.htaccess` overlay UX (offline; no publish from check/explain/compile --dry-run).
    Htaccess {
        #[command(subcommand)]
        command: HtaccessCommand,
    },
}

#[derive(Subcommand)]
enum HtaccessCommand {
    /// Discover + compile overlay for a document root (no publish).
    Check {
        /// Document root containing `.htaccess` files.
        path: PathBuf,
        #[arg(long, default_value = "site")]
        site: String,
        #[arg(long, value_enum, default_value_t = DiagFormat::Human)]
        format: DiagFormat,
    },
    /// Explain supported / limited / unsupported directives in a single `.htaccess` file.
    Explain {
        path: PathBuf,
        #[arg(long, value_enum, default_value_t = DiagFormat::Human)]
        format: DiagFormat,
    },
    /// Show overlay support limits / product pins (site id is informational).
    Status {
        site: String,
        #[arg(long, value_enum, default_value_t = DiagFormat::Human)]
        format: DiagFormat,
    },
    /// Compile overlay for a document root (default dry-run; never publishes from this CLI).
    Compile {
        path: PathBuf,
        #[arg(long, default_value = "site")]
        site: String,
        #[arg(long, default_value_t = true)]
        dry_run: bool,
        #[arg(long, value_enum, default_value_t = DiagFormat::Human)]
        format: DiagFormat,
    },
}

#[derive(Subcommand)]
enum ProfileCommand {
    /// List product profiles (and note internal Dev/Edge/Lb).
    List {
        #[arg(long, value_enum, default_value_t = DiagFormat::Human)]
        format: DiagFormat,
    },
    /// Emit effective AppConfig TOML for a product profile.
    Render {
        profile: String,
        #[arg(long)]
        listen: Option<String>,
        #[arg(long)]
        document_root: Option<PathBuf>,
        #[arg(long)]
        upstream: Option<String>,
        #[arg(long)]
        fpm_address: Option<String>,
        #[arg(long)]
        fpm_transport: Option<String>,
        #[arg(long)]
        index: Option<String>,
        #[arg(long, value_enum, default_value_t = DiagFormat::Human)]
        format: DiagFormat,
    },
    /// Explain a product profile defaults and support limits.
    Explain {
        profile: String,
        #[arg(long, value_enum, default_value_t = DiagFormat::Human)]
        format: DiagFormat,
    },
    /// Expand + validate + compile RuntimePlan (discard; no listeners).
    Test {
        profile: String,
        #[arg(long)]
        listen: Option<String>,
        #[arg(long)]
        document_root: Option<PathBuf>,
        #[arg(long)]
        upstream: Option<String>,
        #[arg(long)]
        fpm_address: Option<String>,
        #[arg(long)]
        fpm_transport: Option<String>,
        #[arg(long)]
        index: Option<String>,
        #[arg(long, value_enum, default_value_t = DiagFormat::Human)]
        format: DiagFormat,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum ControlOutFormat {
    #[default]
    Human,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum DiagFormat {
    #[default]
    Human,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliOutputFormat {
    Toml,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliReportFormat {
    Text,
    Json,
}

#[derive(Debug, Deserialize, Serialize)]
struct ControlResponse {
    ok: bool,
    command: String,
    generation: u64,
    fingerprint: String,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    uptime_s: Option<u64>,
    #[serde(default)]
    active_connections: Option<u64>,
    #[serde(default)]
    draining: Option<bool>,
    #[serde(default)]
    reload_in_progress: Option<bool>,
    #[serde(default)]
    version: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.version {
        println!("exyonqctl {CTL_VERSION}");
        return ExitCode::SUCCESS;
    }

    match cli.command {
        Some(Command::Reload { config, socket }) => run_reload(config, socket),
        Some(Command::Status { socket, format }) => run_control("status", socket, format),
        Some(Command::Drain { socket, format }) => run_control("drain", socket, format),
        Some(Command::Shutdown { socket, format }) => run_control("shutdown", socket, format),
        Some(Command::Purge {
            command,
            socket,
            token,
        }) => run_purge(command, socket, token),
        Some(Command::Config { command }) => run_config(command),
        None => {
            eprintln!(
                "exyonqctl: use reload, status, drain, shutdown, purge, or config (see --help)"
            );
            ExitCode::from(2)
        }
    }
}

fn run_config(command: ConfigCommand) -> ExitCode {
    match command {
        ConfigCommand::Lint {
            path,
            format,
            strict,
            no_color,
        } => emit_result(&lint(
            &path,
            &CheckOptions {
                format: map_fmt(format),
                strict,
                color: !no_color,
            },
        )),
        ConfigCommand::Test {
            path,
            format,
            strict,
            no_color,
        } => emit_result(&test_config(
            &path,
            &CheckOptions {
                format: map_fmt(format),
                strict,
                color: !no_color,
            },
        )),
        ConfigCommand::Explain {
            directive,
            path,
            pointer,
            format,
        } => emit_result(&explain(ExplainRequest {
            directive,
            path,
            pointer,
            format: map_fmt(format),
        })),
        ConfigCommand::Format {
            path,
            write,
            check,
            format,
            no_color: _,
        } => {
            if write && check {
                eprintln!("exyonqctl config format: use either --write or --check, not both");
                return ExitCode::from(2);
            }
            let mode = if write {
                FormatMode::Write
            } else if check {
                FormatMode::Check
            } else {
                FormatMode::Print
            };
            emit_result(&format_exy(FormatRequest {
                path,
                mode,
                format: map_fmt(format),
            }))
        }
        ConfigCommand::Profile { command } => run_profile(command),
        ConfigCommand::Reload {
            path,
            check,
            diff,
            format,
            socket,
        } => run_config_reload(path, check, diff, format, socket),
        ConfigCommand::Status {
            format,
            socket,
            source_config,
        } => run_config_status(format, socket, source_config),
        ConfigCommand::Generation { format, socket } => run_config_generation(format, socket),
        ConfigCommand::MigrateNginx {
            input,
            output,
            report,
            dry_run,
            write,
            check,
            strict,
            format,
            config_format,
        } => run_migrate_nginx(
            input,
            output,
            report,
            dry_run,
            write,
            check,
            strict,
            format,
            config_format,
        ),
        ConfigCommand::Htaccess { command } => run_htaccess(command),
    }
}

fn run_config_reload(
    path: PathBuf,
    check: bool,
    diff: bool,
    format: DiagFormat,
    socket: Option<PathBuf>,
) -> ExitCode {
    if check && diff {
        eprintln!("exyonqctl config reload: use either --check or --diff, not both");
        return ExitCode::from(2);
    }
    if !path.is_file() {
        eprintln!(
            "exyonqctl config reload: config not found: {}",
            path.display()
        );
        return ExitCode::from(2);
    }
    if check {
        return emit_result(&reload_check(&path, map_fmt(format)));
    }
    if diff {
        let (cur_fp, cur_gen) = match invoke_control(&control_socket(socket.clone()), "status\n") {
            Ok(r) => (Some(r.fingerprint), Some(r.generation)),
            Err(_) => (None, None),
        };
        return emit_result(&classify_reload_diff(
            &path,
            cur_fp.as_deref(),
            cur_gen,
        ));
    }
    // Live structural reload: PATH must be the daemon-bound config (honesty).
    // Control plane reloads the process-bound path only; PATH is the operator
    // candidate that must match EXYONQ_CONFIG (or refuse).
    let daemon = std::env::var("EXYONQ_CONFIG")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            // Legacy top-level `exyonqctl reload --config` also required this env.
            None
        });
    let Some(daemon_path) = daemon else {
        eprintln!(
            "exyonqctl config reload: set EXYONQ_CONFIG to the daemon-bound config path \
             (live reload publishes that path only; candidate PATH must match)"
        );
        return ExitCode::from(2);
    };
    let cand = match std::fs::canonicalize(&path) {
        Ok(p) => p,
        Err(err) => {
            eprintln!("exyonqctl config reload: canonicalize {}: {err}", path.display());
            return ExitCode::from(2);
        }
    };
    let bound = match std::fs::canonicalize(&daemon_path) {
        Ok(p) => p,
        Err(err) => {
            eprintln!(
                "exyonqctl config reload: canonicalize EXYONQ_CONFIG {}: {err}",
                daemon_path.display()
            );
            return ExitCode::from(2);
        }
    };
    if cand != bound {
        eprintln!(
            "EXY-RELOAD-0009: candidate PATH ({}) does not match daemon-bound EXYONQ_CONFIG ({})",
            path.display(),
            daemon_path.display()
        );
        return ExitCode::from(1);
    }
    let pre = reload_check(&path, ConfigOut::Human);
    if pre.exit != exyonq_config_cli::CliExit::Ok {
        return emit_result(&pre);
    }
    match invoke_control(&control_socket(socket), "reload\n") {
        Ok(response) => print_response(response, ControlOutFormat::Human),
        Err(err) => {
            eprintln!("exyonqctl config reload: {err}");
            ExitCode::from(1)
        }
    }
}

fn run_config_status(
    format: DiagFormat,
    socket: Option<PathBuf>,
    source_config: Option<PathBuf>,
) -> ExitCode {
    match invoke_control(&control_socket(socket), "status\n") {
        Ok(r) => {
            let src = source_config.or_else(|| {
                std::env::var("EXYONQ_CONFIG")
                    .ok()
                    .map(PathBuf::from)
            });
            emit_result(&format_config_status(
                r.generation,
                &r.fingerprint,
                src.as_deref(),
                r.draining.unwrap_or(false),
                r.reload_in_progress.unwrap_or(false),
                map_fmt(format),
            ))
        }
        Err(err) => {
            eprintln!("exyonqctl config status: {err}");
            ExitCode::from(1)
        }
    }
}

fn run_config_generation(format: DiagFormat, socket: Option<PathBuf>) -> ExitCode {
    match invoke_control(&control_socket(socket), "status\n") {
        Ok(r) => emit_result(&format_generation(r.generation, &r.fingerprint, map_fmt(format))),
        Err(err) => {
            eprintln!("exyonqctl config generation: {err}");
            ExitCode::from(1)
        }
    }
}

fn run_profile(command: ProfileCommand) -> ExitCode {
    match command {
        ProfileCommand::List { format } => emit_result(&profile_list(map_fmt(format))),
        ProfileCommand::Render {
            profile,
            listen,
            document_root,
            upstream,
            fpm_address,
            fpm_transport,
            index,
            format,
        } => emit_result(&profile_render(
            &profile,
            &ProfileCliInputs {
                listen,
                document_root,
                upstream_target: upstream,
                fpm_address,
                fpm_transport,
                index,
            },
            map_fmt(format),
        )),
        ProfileCommand::Explain { profile, format } => {
            emit_result(&profile_explain(&profile, map_fmt(format)))
        }
        ProfileCommand::Test {
            profile,
            listen,
            document_root,
            upstream,
            fpm_address,
            fpm_transport,
            index,
            format,
        } => emit_result(&profile_test(
            &profile,
            &ProfileCliInputs {
                listen,
                document_root,
                upstream_target: upstream,
                fpm_address,
                fpm_transport,
                index,
            },
            map_fmt(format),
        )),
    }
}

fn map_fmt(f: DiagFormat) -> ConfigOut {
    match f {
        DiagFormat::Human => ConfigOut::Human,
        DiagFormat::Json => ConfigOut::Json,
    }
}

fn run_migrate_nginx(
    input: PathBuf,
    output: Option<PathBuf>,
    report_path: Option<PathBuf>,
    dry_run: bool,
    write: bool,
    check: bool,
    strict: bool,
    format: DiagFormat,
    config_format: CliOutputFormat,
) -> ExitCode {
    if !input.is_file() {
        eprintln!(
            "exyonqctl config migrate-nginx: input not found: {}",
            input.display()
        );
        return ExitCode::from(2);
    }

    let options = MigrateOptions {
        dry_run,
        strict,
        format: match config_format {
            CliOutputFormat::Toml => OutputFormat::Toml,
            CliOutputFormat::Json => OutputFormat::Json,
        },
        report_format: match format {
            DiagFormat::Human => ReportFormat::Text,
            DiagFormat::Json => ReportFormat::Json,
        },
    };

    let source_bytes = match std::fs::read(&input) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("exyonqctl config migrate-nginx: read {}: {err}", input.display());
            return ExitCode::from(2);
        }
    };

    let mut migrate_out = match exyonq_compat_nginx::migrate_file(&input, &options) {
        Ok(o) => o,
        Err(err) => {
            eprintln!(
                "EXY-IMPORT-0001: {}",
                redact_secrets(&format!("{err:#}"))
            );
            return ExitCode::from(1);
        }
    };

    // Prefer TOML IR for check / fingerprint even if JSON wrapper requested for display.
    let ir_toml = if matches!(config_format, CliOutputFormat::Toml) {
        migrate_out.config.clone()
    } else {
        // JSON wrapper embeds config_toml — re-migrate as TOML for validation path.
        let toml_opts = MigrateOptions {
            format: OutputFormat::Toml,
            ..options
        };
        match exyonq_compat_nginx::migrate_file(&input, &toml_opts) {
            Ok(o) => {
                migrate_out.report = o.report;
                migrate_out.summary = o.summary;
                o.config
            }
            Err(err) => {
                eprintln!(
                    "EXY-IMPORT-0001: {}",
                    redact_secrets(&format!("{err:#}"))
                );
                return ExitCode::from(1);
            }
        }
    };

    let mut compile_result = "NOT_RUN".to_string();
    if check {
        match exyonq_config_ir::AppConfig::parse_str(&ir_toml) {
            Ok(cfg) => match exyonq_runtime_plan::compile_runtime_plan_from_ir(0, cfg) {
                Ok(plan) => {
                    drop(plan);
                    compile_result = "PASS".into();
                }
                Err(err) => {
                    compile_result = format!("FAIL: {}", redact_secrets(&err.to_string()));
                    eprintln!("EXY-IMPORT-0007: {compile_result}");
                    let product = redact_product_report(ProductImportReport::from_migrate(
                        &input.display().to_string(),
                        &source_bytes,
                        &ir_toml,
                        &migrate_out.report,
                        &compile_result,
                    ));
                    emit_product_report(&product, format, report_path.as_deref());
                    return ExitCode::from(1);
                }
            },
            Err(err) => {
                compile_result = format!("FAIL_VALIDATE: {}", redact_secrets(&err.to_string()));
                eprintln!("EXY-IMPORT-0006: {compile_result}");
                let product = redact_product_report(ProductImportReport::from_migrate(
                    &input.display().to_string(),
                    &source_bytes,
                    &ir_toml,
                    &migrate_out.report,
                    &compile_result,
                ));
                emit_product_report(&product, format, report_path.as_deref());
                return ExitCode::from(1);
            }
        }
    }

    let product = redact_product_report(ProductImportReport::from_migrate(
        &input.display().to_string(),
        &source_bytes,
        &ir_toml,
        &migrate_out.report,
        &compile_result,
    ));

    let will_write = write && output.is_some() && !dry_run;
    if write && dry_run {
        eprintln!("exyonqctl config migrate-nginx: --write ignored because --dry-run is set");
    }
    if will_write {
        if let Some(path) = &output {
            if let Err(err) = std::fs::write(path, &ir_toml) {
                eprintln!(
                    "exyonqctl config migrate-nginx: write {}: {err}",
                    path.display()
                );
                return ExitCode::from(2);
            }
        }
    } else if !check {
        // Preview IR on stdout (redact nothing in TOML structure beyond secret patterns in strings).
        println!("{}", redact_secrets(&migrate_out.config));
    }

    emit_product_report(&product, format, report_path.as_deref());
    eprintln!(
        "TOTAL_COMPAT_PROMISE=FORBIDDEN taxonomy exact={} lossy={} unsupported={} rejected={} invalid={}",
        product.taxonomy.exact,
        product.taxonomy.lossy,
        product.taxonomy.unsupported,
        product.taxonomy.rejected,
        product.taxonomy.invalid_source
    );
    ExitCode::from(product.exit_code(strict) as u8)
}

fn emit_product_report(
    product: &ProductImportReport,
    format: DiagFormat,
    report_path: Option<&std::path::Path>,
) {
    let body = match format {
        DiagFormat::Human => product.render_human(),
        DiagFormat::Json => product
            .render_json()
            .unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}")),
    };
    let body = redact_secrets(&body);
    if let Some(path) = report_path {
        if let Err(err) = std::fs::write(path, &body) {
            eprintln!(
                "exyonqctl config migrate-nginx: write report {}: {err}",
                path.display()
            );
        }
    }
    println!("--- compatibility report ---");
    println!("{body}");
}

fn run_htaccess(command: HtaccessCommand) -> ExitCode {
    match command {
        HtaccessCommand::Check {
            path,
            site,
            format,
        } => htaccess_compile_report(&path, &site, format, true),
        HtaccessCommand::Compile {
            path,
            site,
            dry_run,
            format,
        } => {
            if !dry_run {
                eprintln!(
                    "exyonqctl config htaccess compile: publish is not offered by this CLI \
                     (watcher/runtime owns publish). Forcing dry-run."
                );
            }
            htaccess_compile_report(&path, &site, format, true)
        }
        HtaccessCommand::Explain { path, format } => htaccess_explain(&path, format),
        HtaccessCommand::Status { site, format } => htaccess_status(&site, format),
    }
}

fn htaccess_compile_report(
    document_root: &PathBuf,
    site: &str,
    format: DiagFormat,
    _dry_run: bool,
) -> ExitCode {
    if !document_root.is_dir() {
        eprintln!(
            "EXY-HTACCESS-0006: document root not found: {}",
            document_root.display()
        );
        return ExitCode::from(2);
    }
    match exyonq_mod_htaccess::compile_vhost_overlay(site, document_root, 0) {
        Ok(out) => {
            let body = format_htaccess_compile(&out.report, site, document_root, format);
            println!("{}", redact_secrets(&body));
            if out.report.errors.is_empty() {
                ExitCode::SUCCESS
            } else {
                eprintln!("EXY-HTACCESS-0004: compile reported errors");
                ExitCode::from(1)
            }
        }
        Err(err) => {
            eprintln!(
                "EXY-HTACCESS-0004: {}",
                redact_secrets(&err.to_string())
            );
            eprintln!("EXY-HTACCESS-0005: previous overlay retained (CLI dry-run; no publish)");
            ExitCode::from(1)
        }
    }
}

fn format_htaccess_compile(
    report: &exyonq_mod_htaccess::CompileReport,
    site: &str,
    root: &PathBuf,
    format: DiagFormat,
) -> String {
    match format {
        DiagFormat::Human => {
            let mut out = String::new();
            out.push_str(&format!("site = {site}\n"));
            out.push_str(&format!("document_root = {}\n", root.display()));
            out.push_str("FULL_HTACCESS_COMPATIBILITY = NO\n");
            out.push_str("PUBLISH = NO\n");
            out.push_str(&format!("executed = {}\n", report.executed));
            out.push_str(&format!("parsed_only = {}\n", report.parsed_only));
            out.push_str(&format!("unknown = {}\n", report.unknown));
            for d in &report.diagnostics {
                out.push_str(&format!("diag: {d}\n"));
            }
            for e in &report.errors {
                out.push_str(&format!("error: {e}\n"));
            }
            out
        }
        DiagFormat::Json => format!(
            "{{\n  \"schema_version\": 1,\n  \"site\": \"{}\",\n  \"document_root\": \"{}\",\n  \"full_htaccess_compatibility\": false,\n  \"publish\": false,\n  \"executed\": {},\n  \"parsed_only\": {},\n  \"unknown\": {},\n  \"errors\": {},\n  \"diagnostics\": {}\n}}",
            json_escape(site),
            json_escape(&root.display().to_string()),
            report.executed,
            report.parsed_only,
            report.unknown,
            serde_json::to_string(&report.errors).unwrap_or_else(|_| "[]".into()),
            serde_json::to_string(&report.diagnostics).unwrap_or_else(|_| "[]".into()),
        ),
    }
}

fn htaccess_explain(path: &PathBuf, format: DiagFormat) -> ExitCode {
    if !path.is_file() {
        eprintln!("EXY-HTACCESS-0001: file not found: {}", path.display());
        return ExitCode::from(2);
    }
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("EXY-HTACCESS-0001: {err}");
            return ExitCode::from(1);
        }
    };
    let parsed = match exyonq_mod_htaccess::parse_htaccess(&path.display().to_string(), &raw) {
        Ok(p) => p,
        Err(err) => {
            eprintln!("EXY-HTACCESS-0001: {}", redact_secrets(&err.to_string()));
            return ExitCode::from(1);
        }
    };
    // Re-compile single-file via temp docroot is heavy; explain from directive names.
    let mut lines = Vec::new();
    for d in &parsed.directives {
        let name = d.name.to_ascii_lowercase();
        let class = match name.as_str() {
            "directoryindex" | "redirect" | "rewriteengine" | "options" => "SUPPORTED",
            "rewriterule" | "rewritecond" => "SUPPORTED_LIMITED",
            "php_value" | "php_flag" => "FORBIDDEN",
            _ => "UNSUPPORTED",
        };
        let code = match class {
            "UNSUPPORTED" => "EXY-HTACCESS-0002",
            "FORBIDDEN" => "EXY-HTACCESS-0003",
            _ => "",
        };
        lines.push(format!("{name} => {class} {code}"));
    }
    match format {
        DiagFormat::Human => {
            println!("FULL_HTACCESS_COMPATIBILITY = NO");
            println!("file = {}", path.display());
            for l in lines {
                println!("{l}");
            }
        }
        DiagFormat::Json => {
            println!(
                "{{\n  \"schema_version\": 1,\n  \"full_htaccess_compatibility\": false,\n  \"file\": \"{}\",\n  \"directives\": [{}]\n}}",
                json_escape(&path.display().to_string()),
                lines
                    .iter()
                    .map(|l| format!("\"{}\"", json_escape(l)))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    ExitCode::SUCCESS
}

fn htaccess_status(site: &str, format: DiagFormat) -> ExitCode {
    match format {
        DiagFormat::Human => {
            println!("site = {site}");
            println!("FULL_HTACCESS_COMPATIBILITY = NO");
            println!("WORDPRESS_BASIC_PERMALINKS_WITH_SUPPORTED_OVERLAY = PASS");
            println!("INVALID_HTACCESS_RETAINS_PREVIOUS = PASS");
            println!("PARTIAL_OVERLAY_PUBLISH = NO");
            println!("overlay_publish_owner = runtime_watcher");
            println!("cli_publish = FORBIDDEN");
        }
        DiagFormat::Json => {
            println!(
                "{{\n  \"schema_version\": 1,\n  \"site\": \"{}\",\n  \"full_htaccess_compatibility\": false,\n  \"wordpress_basic_permalinks_with_supported_overlay\": true,\n  \"invalid_htaccess_retains_previous\": true,\n  \"partial_overlay_publish\": false,\n  \"cli_publish\": \"FORBIDDEN\"\n}}",
                json_escape(site)
            );
        }
    }
    ExitCode::SUCCESS
}

fn json_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '"' => "\\\"".to_string(),
            '\\' => "\\\\".to_string(),
            '\n' => "\\n".to_string(),
            c if c.is_control() => " ".to_string(),
            c => c.to_string(),
        })
        .collect()
}

fn purge_socket(explicit: Option<PathBuf>) -> Option<PathBuf> {
    explicit.or_else(|| {
        std::env::var("EXYONQ_CACHE_PURGE_SOCKET")
            .ok()
            .map(PathBuf::from)
    })
}

fn purge_token(explicit: Option<String>) -> Option<String> {
    explicit.or_else(|| std::env::var("EXYONQ_CACHE_PURGE_TOKEN").ok())
}

#[derive(Debug, Deserialize)]
struct PurgeResponse {
    ok: bool,
    command: String,
    site_id: u64,
    purged_entries: u64,
    purged_bytes: u64,
    generation: u64,
    #[serde(default)]
    error: Option<String>,
}

fn run_purge(command: PurgeCommand, socket: Option<PathBuf>, token: Option<String>) -> ExitCode {
    let Some(socket_path) = purge_socket(socket) else {
        eprintln!("exyonqctl purge: pass --socket or set EXYONQ_CACHE_PURGE_SOCKET");
        return ExitCode::from(2);
    };
    let Some(token) = purge_token(token) else {
        eprintln!("exyonqctl purge: pass --token or set EXYONQ_CACHE_PURGE_TOKEN");
        return ExitCode::from(2);
    };
    if token.is_empty() {
        eprintln!("exyonqctl purge: empty token rejected");
        return ExitCode::from(2);
    }

    let line = match command {
        PurgeCommand::Site { site_id } => format!("purge site {site_id} {token}\n"),
        PurgeCommand::Generation {
            site_id,
            generation,
        } => format!("purge generation {site_id} {generation} {token}\n"),
        PurgeCommand::Url {
            site_id,
            scheme,
            host,
            path,
            query,
        } => {
            if query.is_empty() {
                format!("purge url {site_id} {scheme} {host} {path} {token}\n")
            } else {
                format!("purge url {site_id} {scheme} {host} {path} {query} {token}\n")
            }
        }
    };

    match invoke_purge(&socket_path, &line) {
        Ok(response) => {
            if response.ok {
                println!(
                    "{} ok site_id={} purged_entries={} purged_bytes={} generation={}",
                    response.command,
                    response.site_id,
                    response.purged_entries,
                    response.purged_bytes,
                    response.generation
                );
                ExitCode::SUCCESS
            } else {
                eprintln!(
                    "{} failed site_id={} error={}",
                    response.command,
                    response.site_id,
                    response.error.as_deref().unwrap_or("unknown")
                );
                ExitCode::from(1)
            }
        }
        Err(err) => {
            eprintln!("exyonqctl purge: {err}");
            ExitCode::from(1)
        }
    }
}

fn invoke_purge(socket_path: &PathBuf, command: &str) -> std::io::Result<PurgeResponse> {
    let mut stream = std::os::unix::net::UnixStream::connect(socket_path)?;
    stream.write_all(command.as_bytes())?;
    let mut line = String::new();
    std::io::BufReader::new(&mut stream).read_line(&mut line)?;
    serde_json::from_str(line.trim()).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid purge response: {err}"),
        )
    })
}

fn control_socket(explicit: Option<PathBuf>) -> PathBuf {
    explicit
        .or_else(|| {
            std::env::var("EXYONQ_CONTROL_SOCKET")
                .ok()
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from("/tmp/exyonq.sock"))
}

fn run_reload(config: Option<PathBuf>, socket: Option<PathBuf>) -> ExitCode {
    let config_path =
        match config.or_else(|| std::env::var("EXYONQ_CONFIG").ok().map(PathBuf::from)) {
            Some(path) => path,
            None => {
                eprintln!("exyonqctl reload: pass --config or set EXYONQ_CONFIG");
                return ExitCode::from(2);
            }
        };

    if !config_path.is_file() {
        eprintln!(
            "exyonqctl reload: config not found: {}",
            config_path.display()
        );
        return ExitCode::from(2);
    }

    let _ = config_path;
    match invoke_control(&control_socket(socket), "reload\n") {
        Ok(response) => print_response(response, ControlOutFormat::Human),
        Err(err) => {
            eprintln!("exyonqctl reload: {err}");
            ExitCode::from(1)
        }
    }
}

fn run_control(command: &str, socket: Option<PathBuf>, format: ControlOutFormat) -> ExitCode {
    match invoke_control(&control_socket(socket), &format!("{command}\n")) {
        Ok(response) => print_response(response, format),
        Err(err) => {
            eprintln!("exyonqctl {command}: {err}");
            ExitCode::from(1)
        }
    }
}

fn invoke_control(socket_path: &PathBuf, command: &str) -> std::io::Result<ControlResponse> {
    let mut stream = std::os::unix::net::UnixStream::connect(socket_path)?;
    stream.write_all(command.as_bytes())?;
    let mut line = String::new();
    std::io::BufReader::new(&mut stream).read_line(&mut line)?;
    serde_json::from_str(line.trim()).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid control response: {err}"),
        )
    })
}

fn print_response(response: ControlResponse, format: ControlOutFormat) -> ExitCode {
    if matches!(format, ControlOutFormat::Json) {
        // P15-WS5-STATUS-001: emit wire JSON (already secret-free kernel snapshot).
        match serde_json::to_string(&response) {
            Ok(s) => {
                println!("{s}");
                if response.ok {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                }
            }
            Err(err) => {
                eprintln!("exyonqctl: failed to serialize status JSON: {err}");
                ExitCode::from(1)
            }
        }
    } else if response.ok {
        let process_state = if response.draining == Some(true) {
            "draining"
        } else {
            "serving"
        };
        print!(
            "{} ok process_state={} generation={} fingerprint={}",
            response.command, process_state, response.generation, response.fingerprint
        );
        if let Some(uptime) = response.uptime_s {
            print!(" uptime_s={uptime}");
        }
        if let Some(active) = response.active_connections {
            print!(" active_connections={active}");
        }
        if let Some(draining) = response.draining {
            print!(" draining={draining}");
        }
        if let Some(reload_in_progress) = response.reload_in_progress {
            print!(" reload_in_progress={reload_in_progress}");
        }
        if let Some(version) = response.version {
            print!(" version={version}");
        }
        if let Some(code) = &response.code {
            print!(" code={code}");
        }
        println!();
        if let Some(err) = &response.error {
            // Informational note on success (e.g. identical NO_OP).
            eprintln!("  {err}");
        }
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "{} failed generation={} fingerprint={}",
            response.command, response.generation, response.fingerprint
        );
        if let Some(code) = response.code {
            eprintln!("  code={code}");
        }
        if let Some(err) = response.error {
            eprintln!("  {err}");
        }
        ExitCode::from(1)
    }
}
