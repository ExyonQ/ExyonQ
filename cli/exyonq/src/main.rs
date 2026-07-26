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

mod allocator;
mod l2_coord_lab;

use anyhow::Context;
use clap::{Parser, Subcommand};
use exyonq_config_cli::{
    emit_result, explain, format_exy, lint, CheckOptions, CliExit, ExplainRequest, FormatMode,
    FormatRequest, OutputFormat as ConfigOut,
};
use exyonq_config_ir::AppConfig;
use exyonq_config_merge::{load_with_includes, Profile};
use exyonq_config_schema::{migrate_v1_to_v2_toml, write_ir_schema};
use exyonq_config_surface::{compile_serverfile, CompileOptions};
use exyonq_core::FcgiRuntimeRegistration;
use exyonq_platform_linux::ensure_composition_link;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use tracing::info;
use tracing_subscriber::EnvFilter;

/// Honest product version metadata (P1.5-WS6). No builder paths or secrets.
#[cfg(feature = "allocator-jemalloc")]
const CLI_VERSION: &str = concat!(
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
    "\nallocator=jemalloc"
);

#[cfg(not(feature = "allocator-jemalloc"))]
const CLI_VERSION: &str = concat!(
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
    name = "exyonq",
    version = CLI_VERSION,
    long_version = CLI_VERSION,
    about = "ExyonQ modular reverse proxy"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the HTTP server from native IR (TOML) or Serverfile (.exy).
    Serve {
        #[arg(short, long)]
        config: PathBuf,
    },
    /// Validate native IR (TOML).
    Validate {
        #[arg(short, long)]
        config: PathBuf,
    },
    /// Compile Serverfile (.exy) to IR TOML.
    Compile {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        check: bool,
        #[arg(long, value_enum)]
        profile: Option<IrProfile>,
    },
    /// Migrate IR v1 TOML to v2.
    MigrateConfig {
        #[arg(short, long)]
        config: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Format a Serverfile.
    Fmt {
        input: PathBuf,
        #[arg(short, long)]
        write: bool,
    },
    /// Explain a Serverfile directive.
    Explain { directive: String },
    /// Export JSON Schema for IR.
    ExportSchema {
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Run the Fase 0 HTTP reverse-proxy spike.
    Spike {
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: SocketAddr,
        #[arg(long, default_value = "http://127.0.0.1:9000")]
        upstream: String,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum IrProfile {
    Dev,
    Edge,
    Lb,
}

impl From<IrProfile> for Profile {
    fn from(value: IrProfile) -> Self {
        match value {
            IrProfile::Dev => Profile::Dev,
            IrProfile::Edge => Profile::Edge,
            IrProfile::Lb => Profile::Lb,
        }
    }
}

fn tokio_worker_threads() -> usize {
    if let Ok(raw) = std::env::var("EXYONQ_WORKER_THREADS") {
        if let Ok(n) = raw.parse::<usize>() {
            return n.clamp(1, 64);
        }
    }
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .clamp(1, 16)
}

fn init_tracing() -> anyhow::Result<()> {
    let filter = EnvFilter::from_default_env().add_directive("info".parse()?);
    if std::env::var("EXYONQ_LOG_FORMAT")
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
    {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    // PS3A: composition root links `exyonq-platform-linux` → `exyonq-core` (D1).
    // Workers remain in core until authorized extraction follow-up.
    ensure_composition_link();
    init_tracing()?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(tokio_worker_threads())
        .max_blocking_threads(4)
        .enable_all()
        .build()?;
    rt.block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // Keep compile-time identity referenced (also embedded in `--version`).
    debug_assert!(
        CLI_VERSION.contains(allocator::ALLOCATOR_NAME),
        "CLI_VERSION must match allocator::ALLOCATOR_NAME"
    );

    match cli.command {
        Some(Commands::Serve { config }) => {
            info!(allocator = allocator::ALLOCATOR_NAME, "allocator identity");
            let app_config = load_for_serve(&config)?;
            std::env::set_var("EXYONQ_CONFIG", config.display().to_string());
            if std::env::var("EXYONQ_CONTROL_SOCKET").is_err() {
                std::env::set_var("EXYONQ_CONTROL_SOCKET", "/tmp/exyonq.sock");
            }
            exyonq_core::register_fcgi_cache_hooks(exyonq_core::fcgi_cache_hooks_from_module(
                exyonq_mod_fastcgi::serve_fastcgi_with_cache_hook as exyonq_core::ServeFn,
                exyonq_core::FcgiCacheMetricsFns {
                    hits: exyonq_mod_fastcgi::cache_fcgi_hits_total,
                    misses: exyonq_mod_fastcgi::cache_fcgi_misses_total,
                    insertions: exyonq_mod_fastcgi::cache_fcgi_insertions_total,
                    rejections: exyonq_mod_fastcgi::cache_fcgi_rejections_total,
                    reset: exyonq_mod_fastcgi::reset_fcgi_cache_metrics_for_tests,
                },
            ))
            .map_err(|_| anyhow::anyhow!("FastCGI cache hooks already registered"))?;

            // KD4.7: module-owned script resolution contract (register-once).
            exyonq_mod_fastcgi::bridge::register_script_resolver()
                .map_err(|_| anyhow::anyhow!("FastCGI script resolver already registered"))?;
            if let Some(registration) = resolve_fcgi_registration(&app_config) {
                exyonq_mod_fastcgi::register_with_core(registration, |service| {
                    exyonq_core::register_fcgi_dispatch_service(service)
                })
                .map_err(|err| match err {
                    exyonq_core::FcgiRegisterError::AlreadyRegistered => {
                        anyhow::anyhow!("FastCGI executor already registered")
                    }
                    exyonq_core::FcgiRegisterError::Poisoned => {
                        anyhow::anyhow!("FastCGI executor mutex poisoned")
                    }
                    exyonq_core::FcgiRegisterError::InvalidCapacity => {
                        anyhow::anyhow!("FastCGI pool max_concurrency invalid")
                    }
                })?;
            } else if !app_config.pools_fcgi.is_empty() {
                tracing::warn!(
                    "FastCGI pools declared but no resolvable unix path or literal TCP address; \
                     set fcgi_pool.address + transport or EXYONQ_FCGI_SOCKET — FastCGI routes return 501"
                );
            }
            let static_runtime = std::sync::Arc::new(exyonq_mod_static::StaticRuntime::new());
            let static_service: std::sync::Arc<dyn exyonq_core::StaticDispatchService> =
                static_runtime.clone();
            exyonq_core::register_static_dispatch_service(static_service).map_err(
                |err| match err {
                    exyonq_core::StaticRegisterError::AlreadyRegistered => {
                        anyhow::anyhow!("Static dispatch service already registered")
                    }
                    exyonq_core::StaticRegisterError::Poisoned => {
                        anyhow::anyhow!("Static dispatch service mutex poisoned")
                    }
                    exyonq_core::StaticRegisterError::InvalidRootSlot => {
                        anyhow::anyhow!("Static root slot invalid")
                    }
                },
            )?;
            exyonq_mod_static::install_kernel_hooks(static_runtime);
            let proxy_reg = exyonq_mod_proxy::registration_with_default_runtime();
            exyonq_core::register_proxy_dispatch_service(proxy_reg.service).map_err(
                |err| match err {
                    exyonq_core::ProxyRegisterError::AlreadyRegistered => {
                        anyhow::anyhow!("Proxy dispatch service already registered")
                    }
                    exyonq_core::ProxyRegisterError::Poisoned => {
                        anyhow::anyhow!("Proxy dispatch service mutex poisoned")
                    }
                },
            )?;
            start_htaccess_overlay(&app_config)?;
            exyonq_ops_runtime::register_control_plane()
                .map_err(|_| anyhow::anyhow!("control plane service already registered"))?;
            exyonq_discovery_runtime::register_discovery_runtime()
                .map_err(|_| anyhow::anyhow!("discovery runtime service already registered"))?;
            exyonq_acme::register_acme_integration()
                .map_err(|_| anyhow::anyhow!("ACME integration service already registered"))?;
            exyonq_metrics::register_observability_runtime()
                .map_err(|_| anyhow::anyhow!("observability runtime already registered"))?;
            exyonq_reload_runtime::register_reload_runtime()
                .map_err(|_| anyhow::anyhow!("reload runtime already registered"))?;
            l2_coord_lab::maybe_install_l2_coord_lab(&app_config)?;
            info!(path = %config.display(), listen = %app_config.primary_listen_addr()?);
            exyonq_core::server::run(app_config).await?;
        }
        Some(Commands::Validate { config }) => {
            eprintln!(
                "warning: `exyonq validate` is transitional; prefer `exyonqctl config lint` \
                 (or `exyonqctl config test` for RuntimePlan compile)"
            );
            // Historical semantics: parse+validate only (no RuntimePlan) → lint.
            let result = lint(
                &config,
                &CheckOptions {
                    format: ConfigOut::Human,
                    strict: false,
                    color: true,
                },
            );
            emit_result(&result);
            if result.exit == CliExit::Ok {
                println!("config OK: {}", config.display());
            } else {
                std::process::exit(result.exit as u8 as i32);
            }
        }
        Some(Commands::Compile {
            input,
            output,
            check,
            profile,
        }) => {
            eprintln!(
                "warning: `exyonq compile` is transitional; product surface is \
                 `exyonqctl config lint|test` after authoring `.exy` (compile remains \
                 `.exy` → TOML IR tooling)"
            );
            let raw = std::fs::read_to_string(&input)
                .with_context(|| format!("read {}", input.display()))?;
            let opts = CompileOptions {
                profile: profile.map(Into::into),
                generated_comment: true,
            };
            let toml = compile_serverfile(&raw, opts)?;
            if check {
                let expected_path = output
                    .clone()
                    .unwrap_or_else(|| input.with_extension("toml"));
                let expected = std::fs::read_to_string(&expected_path)
                    .with_context(|| format!("read {}", expected_path.display()))?;
                if expected.trim() != toml.trim() {
                    anyhow::bail!("compiled IR differs from {}", expected_path.display());
                }
                println!("compile --check OK");
            } else if let Some(out) = output {
                std::fs::write(&out, &toml)?;
                println!("wrote {}", out.display());
            } else {
                print!("{toml}");
            }
        }
        Some(Commands::MigrateConfig { config, output }) => {
            let raw = std::fs::read_to_string(&config)?;
            let migrated = migrate_v1_to_v2_toml(&raw)?;
            std::fs::write(&output, migrated)?;
            println!("migrated {} -> {}", config.display(), output.display());
        }
        Some(Commands::Fmt { input, write }) => {
            eprintln!(
                "warning: `exyonq fmt` is transitional; prefer `exyonqctl config format`"
            );
            let mode = if write {
                FormatMode::Write
            } else {
                FormatMode::Print
            };
            let result = format_exy(FormatRequest {
                path: input,
                mode,
                format: ConfigOut::Human,
            });
            emit_result(&result);
            if result.exit != CliExit::Ok {
                std::process::exit(result.exit as u8 as i32);
            }
        }
        Some(Commands::Explain { directive }) => {
            eprintln!(
                "warning: `exyonq explain` is transitional; prefer `exyonqctl config explain`"
            );
            let result = explain(ExplainRequest {
                directive: Some(directive),
                path: None,
                pointer: None,
                format: ConfigOut::Human,
            });
            emit_result(&result);
            if result.exit != CliExit::Ok {
                std::process::exit(result.exit as u8 as i32);
            }
        }
        Some(Commands::ExportSchema { output }) => {
            write_ir_schema(&output)?;
            println!("wrote schema {}", output.display());
        }
        Some(Commands::Spike { listen, upstream }) => {
            let upstream = upstream.parse().context("invalid upstream URI")?;
            info!("starting spike proxy");
            exyonq_mod_proxy::run_spike_proxy(listen, upstream).await?;
        }
        None => {}
    }

    Ok(())
}

fn load_for_serve(path: &Path) -> anyhow::Result<AppConfig> {
    if is_serverfile(path) {
        let raw = std::fs::read_to_string(path)?;
        let toml = compile_serverfile(&raw, CompileOptions::default())?;
        return AppConfig::parse_str(&toml).map_err(Into::into);
    }
    load_ir(path)
}

fn load_ir(path: &Path) -> anyhow::Result<AppConfig> {
    if looks_like_v2_include(path) {
        load_with_includes(path).map_err(Into::into)
    } else {
        AppConfig::from_file(path).map_err(Into::into)
    }
}

fn is_serverfile(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "exy")
}

fn looks_like_v2_include(path: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    raw.contains("include =") && raw.contains("config_version = 2")
}

fn resolve_fcgi_registration(app_config: &AppConfig) -> Option<FcgiRuntimeRegistration> {
    if app_config.pools_fcgi.is_empty() {
        return None;
    }
    let mut pool_names: Vec<String> = app_config.pools_fcgi.keys().cloned().collect();
    pool_names.sort();
    let env_socket = std::env::var("EXYONQ_FCGI_SOCKET").ok();
    let address_by_name: std::collections::HashMap<String, String> = app_config
        .pools_fcgi
        .iter()
        .map(|(name, pool)| (name.clone(), pool.address.clone()))
        .collect();
    let transport_by_name: std::collections::HashMap<String, String> = app_config
        .pools_fcgi
        .iter()
        .map(|(name, pool)| (name.clone(), pool.transport.clone()))
        .collect();
    let max_by_name: std::collections::HashMap<String, u32> = app_config
        .pools_fcgi
        .iter()
        .map(|(name, pool)| (name.clone(), pool.max_concurrency))
        .collect();
    let env_max = std::env::var("EXYONQ_FCGI_MAX_CONCURRENCY")
        .ok()
        .and_then(|raw| raw.parse::<u32>().ok());
    let pools = exyonq_mod_fastcgi::resolve_pool_endpoints(
        &pool_names,
        &address_by_name,
        &transport_by_name,
        env_socket.as_deref(),
    );
    if pools.is_empty() {
        return None;
    }
    let capacities = exyonq_mod_fastcgi::resolve_pool_capacities_with_transport(
        &pool_names,
        &address_by_name,
        &transport_by_name,
        &max_by_name,
        env_socket.as_deref(),
        env_max,
    );
    let pools_for_exec = pools.clone();
    #[derive(Clone, Copy)]
    struct PoolConnCfg {
        max_connections: usize,
        idle_ms: u64,
        total_ms: u64,
        checkout_ms: u64,
    }
    let conn_cfg_by_id: std::collections::HashMap<u32, PoolConnCfg> = {
        let mut m = std::collections::HashMap::new();
        for (pool_id, _) in &pools {
            let name = pool_names
                .get(*pool_id as usize)
                .map(String::as_str)
                .unwrap_or("");
            let pool = app_config.pools_fcgi.get(name);
            let max_conc = pool.map(|p| p.max_concurrency).unwrap_or(16);
            let max_conn = pool
                .and_then(|p| p.max_connections)
                .unwrap_or(max_conc) as usize;
            m.insert(
                *pool_id,
                PoolConnCfg {
                    max_connections: max_conn.max(1),
                    idle_ms: pool.map(|p| p.idle_timeout_ms).unwrap_or(30_000).max(1),
                    total_ms: pool.map(|p| p.total_timeout_ms).unwrap_or(30_000).max(1),
                    checkout_ms: pool.map(|p| p.checkout_timeout_ms).unwrap_or(5_000).max(1),
                },
            );
        }
        m
    };
    Some(FcgiRuntimeRegistration {
        executor: std::sync::Arc::new(exyonq_mod_fastcgi::FcgiModuleExecutor::production_pools(
            pools_for_exec,
            move |pool_id| {
                let cfg = conn_cfg_by_id.get(&pool_id).copied().unwrap_or(PoolConnCfg {
                    max_connections: 16,
                    idle_ms: 30_000,
                    total_ms: 30_000,
                    checkout_ms: 5_000,
                });
                exyonq_mod_fastcgi::ConnPoolConfig {
                    max_connections: cfg.max_connections,
                    idle_timeout: std::time::Duration::from_millis(cfg.idle_ms),
                    connect_timeout: exyonq_mod_fastcgi::FCGI_CONNECT_TIMEOUT,
                    total_timeout: std::time::Duration::from_millis(cfg.total_ms),
                    checkout_timeout: std::time::Duration::from_millis(cfg.checkout_ms),
                    ..exyonq_mod_fastcgi::ConnPoolConfig::default()
                }
            },
        )),
        pool_capacities: capacities,
    })
}

fn start_htaccess_overlay(app_config: &AppConfig) -> anyhow::Result<()> {
    use exyonq_core::compile_htaccess_site_bindings;
    use exyonq_mod_htaccess::{spawn_watcher, HtaccessSite, OverlayPublisher};
    use std::sync::Arc;

    let sites: Vec<HtaccessSite> = compile_htaccess_site_bindings(app_config)
        .into_iter()
        .map(|binding| HtaccessSite {
            site_id: binding.site_id,
            document_root: binding.document_root,
        })
        .collect();
    if sites.is_empty() {
        return Ok(());
    }
    let publisher = Arc::new(OverlayPublisher::new(1));
    let overlay_source = Arc::clone(&publisher)
        as Arc<dyn exyonq_module_api::htaccess_runtime::HtaccessOverlaySource>;
    let runtime = Arc::new(exyonq_mod_htaccess::HtaccessRuntime::new(
        overlay_source,
        Arc::new(exyonq_core::CoreHtaccessProbes),
    ));
    exyonq_core::register_htaccess_runtime_service(runtime).map_err(|err| match err {
        exyonq_module_api::htaccess_runtime::HtaccessRegisterError::AlreadyRegistered => {
            anyhow::anyhow!("htaccess runtime already registered")
        }
        exyonq_module_api::htaccess_runtime::HtaccessRegisterError::Poisoned => {
            anyhow::anyhow!("htaccess runtime mutex poisoned")
        }
    })?;
    spawn_watcher(sites, publisher)?;
    Ok(())
}

#[cfg(test)]
mod fcgi_register_tests {
    use super::*;
    use exyonq_config_ir::FcgiPoolConfig;

    fn config_with_pool(address: &str) -> AppConfig {
        let mut config: AppConfig = minimal_app_config();
        config.pools_fcgi.insert(
            "php".into(),
            FcgiPoolConfig {
                name: "php".into(),
                address: address.into(),
                document_root: None,
                max_concurrency: 16,
                transport: "auto".into(),
                total_timeout_ms: 30_000,
                checkout_timeout_ms: 5_000,
                max_connections: None,
                idle_timeout_ms: 30_000,
            },
        );
        config
}

    #[test]
    fn no_env_and_unresolvable_address_skips_registration() {
        // Neither a unix socket path nor a literal SocketAddr → no endpoint.
        let config = config_with_pool("not-a-valid-fcgi-endpoint");
        assert!(resolve_fcgi_registration(&config).is_none());
    }

    #[test]
    fn env_socket_enables_registration_for_first_pool() {
        let config = config_with_pool("127.0.0.1:9000");
        std::env::set_var("EXYONQ_FCGI_SOCKET", "/run/php/php-fpm.sock");
        assert!(resolve_fcgi_registration(&config).is_some());
        std::env::remove_var("EXYONQ_FCGI_SOCKET");
    }

    fn minimal_app_config() -> AppConfig {
        let raw = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = []
"#;
        raw.parse().expect("minimal config")
}
}

#[cfg(test)]
mod htaccess_overlay_tests {
    use super::*;
    use exyonq_config_ir::FcgiPoolConfig;
    use exyonq_core::compile_htaccess_site_bindings;

    #[test]
    fn watcher_sites_include_fastcgi_pool_document_root() {
        let mut config = minimal_app_config();
        config.routes.push(exyonq_config_ir::RouteConfig {
            name: "php".into(),
            r#match: exyonq_config_ir::RouteMatch {
                path: "/".into(),
                host: None,
            },
            upstream: None,
            root: None,
            index: None,
            redirect: None,
            rewrite: None,
            fastcgi: Some("php".into()),
            htaccess: exyonq_config_ir::HtaccessMode::Overlay,
            cache: None,
        });
        config.pools_fcgi.insert(
            "php".into(),
            FcgiPoolConfig {
                name: "php".into(),
                address: "/run/php.sock".into(),
                document_root: Some("/srv/php".into()),
                max_concurrency: 16,
                transport: "auto".into(),
                total_timeout_ms: 30_000,
                checkout_timeout_ms: 5_000,
                max_connections: None,
                idle_timeout_ms: 30_000,
            },
        );
        config.servers[0].routes.push("php".into());
        let bindings = compile_htaccess_site_bindings(&config);
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].document_root.display().to_string(), "/srv/php");
    }

    #[test]
    fn watcher_sites_empty_without_overlay_routes() {
        let mut config = minimal_app_config();
        config.routes.push(exyonq_config_ir::RouteConfig {
            name: "php".into(),
            r#match: exyonq_config_ir::RouteMatch {
                path: "/".into(),
                host: None,
            },
            upstream: None,
            root: None,
            index: None,
            redirect: None,
            rewrite: None,
            fastcgi: Some("php".into()),
            htaccess: exyonq_config_ir::HtaccessMode::Off,
            cache: None,
        });
        assert!(compile_htaccess_site_bindings(&config).is_empty());
    }

    fn minimal_app_config() -> AppConfig {
        let raw = r#"
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = []
"#;
        raw.parse().expect("minimal config")
}
}
