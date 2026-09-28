//! Cap061 subscriber: single `try_init`, PREPARE→COMMIT via filter reload + sink Arc swap.

use anyhow::{bail, Context};
use exyonq_config_ir::{ConsoleStream, LogFormat, LoggingConfig};
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::FormatFields;
use tracing_subscriber::layer::Context as LayerCtx;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::reload;
use tracing_subscriber::{fmt, prelude::*, EnvFilter, Layer, Registry};

static GENERATION: AtomicU64 = AtomicU64::new(0);
static CONTROLLER: OnceLock<Mutex<Controller>> = OnceLock::new();

pub struct ObservabilityGuard {
    generation: u64,
}

impl ObservabilityGuard {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn shutdown(self) {
        if let Some(ctrl) = CONTROLLER.get() {
            if let Ok(mut g) = ctrl.lock() {
                #[cfg(feature = "otel")]
                if let Some(rt) = g.otel_rt.take() {
                    rt.shutdown();
                }
            }
        }
    }
}

struct Controller {
    filter: reload::Handle<EnvFilter, Registry>,
    sinks: Arc<RwLock<SinkGeneration>>,
    #[cfg(feature = "otel")]
    otel: OtelReload,
    #[cfg(feature = "file")]
    file_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
    #[cfg(feature = "file")]
    file_error_counter: Option<tracing_appender::non_blocking::ErrorCounter>,
    #[cfg(feature = "otel")]
    otel_rt: Option<crate::otel::OtelRuntime>,
    cfg: LoggingConfig,
}

type AfterFilter = tracing_subscriber::layer::Layered<reload::Layer<EnvFilter, Registry>, Registry>;
type AfterFanout = tracing_subscriber::layer::Layered<FanoutLayer, AfterFilter>;
/// Historical name: journald now ships via scrubbed FanoutLayer (no separate Layer).
type AfterJournald = AfterFanout;

#[cfg(feature = "otel")]
type OtelReload = reload::Handle<
    Option<
        tracing_opentelemetry::OpenTelemetryLayer<AfterJournald, opentelemetry_sdk::trace::Tracer>,
    >,
    AfterJournald,
>;

/// Active fanout sink snapshot. Cap061 generation identity lives in
/// `GENERATION` / `ObservabilityGuard`, not on this struct (whole-struct swap).
struct SinkGeneration {
    json: bool,
    console: Option<ConsoleStream>,
    #[cfg(feature = "file")]
    file: Option<tracing_appender::non_blocking::NonBlocking>,
    #[cfg(feature = "syslog")]
    syslog: Option<crate::syslog_sink::SyslogLayer>,
    /// Native journald via scrubbed fanout (not a raw tracing_journald field layer).
    #[cfg(feature = "journald")]
    journald_native: bool,
}

struct FanoutLayer {
    sinks: Arc<RwLock<SinkGeneration>>,
}

impl<S> Layer<S> for FanoutLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: LayerCtx<'_, S>) {
        let Ok(sinks) = self.sinks.read() else {
            return;
        };
        let meta = event.metadata();
        let line = if sinks.json {
            let mut fields = serde_json::Map::new();
            fields.insert("timestamp".into(), serde_json::Value::String(now_rfc3339()));
            fields.insert(
                "level".into(),
                serde_json::Value::String(meta.level().to_string()),
            );
            fields.insert(
                "target".into(),
                serde_json::Value::String(meta.target().to_string()),
            );
            let mut visitor = JsonFieldVisitor {
                fields: &mut fields,
            };
            event.record(&mut visitor);
            // Defense-in-depth: scrub string values.
            for v in fields.values_mut() {
                if let serde_json::Value::String(s) = v {
                    *s = crate::redaction::scrub_field_blob(s);
                }
            }
            let mut s = serde_json::Value::Object(fields).to_string();
            s.push('\n');
            s
        } else {
            let mut buf = String::new();
            let field_format = fmt::format::DefaultFields::new();
            let _ = field_format.format_fields(Writer::new(&mut buf), event);
            let buf = crate::redaction::scrub_field_blob(&buf);
            format!("{} {}: {}\n", meta.level(), meta.target(), buf)
        };
        let bytes = line.as_bytes();

        if let Some(stream) = sinks.console {
            let mut out: Box<dyn Write> = match stream {
                ConsoleStream::Stdout => Box::new(io::stdout()),
                ConsoleStream::Stderr => Box::new(io::stderr()),
                ConsoleStream::Split => {
                    if *meta.level() <= tracing::Level::WARN {
                        Box::new(io::stderr())
                    } else {
                        Box::new(io::stdout())
                    }
                }
            };
            let _ = out.write_all(bytes);
            let _ = out.flush();
        }

        #[cfg(feature = "file")]
        if let Some(ref file) = sinks.file {
            let mut f = file.clone();
            if f.write_all(bytes).is_err() {
                crate::bounds::note_sink_error();
            }
        }

        #[cfg(feature = "syslog")]
        if let Some(ref syslog) = sinks.syslog {
            syslog.emit_formatted(meta.level(), &line);
        }

        #[cfg(all(feature = "journald", target_os = "linux"))]
        if sinks.journald_native {
            // `line` is already scrubbed; native journal submission must not see raw fields.
            if crate::journald_sink::emit_line(line.trim_end()).is_err() {
                crate::bounds::note_sink_error();
            }
        }
        #[cfg(all(feature = "journald", not(target_os = "linux")))]
        let _ = sinks.journald_native;
    }
}

struct JsonFieldVisitor<'a> {
    fields: &'a mut serde_json::Map<String, serde_json::Value>,
}

impl tracing::field::Visit for JsonFieldVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let raw = format!("{value:?}").trim_matches('"').to_string();
        let scrubbed = if crate::redaction::is_sensitive_header_name(field.name()) {
            crate::redaction::REDACTED.to_string()
        } else {
            crate::redaction::scrub_field_blob(&raw)
        };
        self.fields.insert(
            field.name().to_string(),
            serde_json::Value::String(scrubbed),
        );
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        let scrubbed = if crate::redaction::is_sensitive_header_name(field.name()) {
            crate::redaction::REDACTED.to_string()
        } else {
            crate::redaction::scrub_field_blob(value)
        };
        self.fields.insert(
            field.name().to_string(),
            serde_json::Value::String(scrubbed),
        );
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields.insert(
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        );
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields.insert(
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        );
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields
            .insert(field.name().to_string(), serde_json::Value::Bool(value));
    }
}

fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d, hh, mm, ss) = civil_from_unix_secs(secs);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Howard Hinnant civil_from_days — UTC Y-M-D h:m:s without extra deps.
fn civil_from_unix_secs(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400) as u32;
    let hh = tod / 3600;
    let mm = (tod % 3600) / 60;
    let ss = tod % 60;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64 + era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, hh, mm, ss)
}

pub fn bootstrap_from_env() -> anyhow::Result<ObservabilityGuard> {
    let mut cfg = LoggingConfig::default();
    if std::env::var("EXYONQ_LOG_FORMAT")
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
    {
        cfg.format = LogFormat::Json;
    }
    if let Ok(level) = std::env::var("EXYONQ_LOG_LEVEL") {
        if !level.trim().is_empty() {
            cfg.level = level;
        }
    }
    install_or_reload(&cfg)
}

pub fn install_from_config(cfg: &LoggingConfig) -> anyhow::Result<ObservabilityGuard> {
    install_or_reload(cfg)
}

pub fn reload_from_config(cfg: &LoggingConfig) -> anyhow::Result<u64> {
    Ok(install_or_reload(cfg)?.generation())
}

fn build_filter(cfg: &LoggingConfig) -> anyhow::Result<EnvFilter> {
    if std::env::var_os("RUST_LOG").is_some() {
        EnvFilter::try_from_default_env().context("invalid RUST_LOG")
    } else {
        EnvFilter::try_new(cfg.level.trim()).context("invalid logging.level")
    }
}

fn feature_gates(cfg: &LoggingConfig) -> anyhow::Result<()> {
    if cfg.syslog.enabled {
        #[cfg(not(feature = "syslog"))]
        bail!("logging.syslog.enabled but exyonq-observability built without `syslog` feature");
    }
    if cfg.journald.enabled {
        #[cfg(not(feature = "journald"))]
        bail!("logging.journald.enabled but exyonq-observability built without `journald` feature");
        #[cfg(not(target_os = "linux"))]
        bail!("logging.journald.enabled is Linux-only (STARTUP_REJECT_CONFIG)");
    }
    if cfg.otel.enabled {
        #[cfg(not(feature = "otel"))]
        bail!("logging.otel.enabled but exyonq-observability built without `otel` feature");
    }
    if cfg.file.enabled {
        #[cfg(not(feature = "file"))]
        bail!("logging.file.enabled but exyonq-observability built without `file` feature");
    }
    if !cfg.console.enabled
        && !cfg.file.enabled
        && !cfg.syslog.enabled
        && !cfg.journald.enabled
        && !cfg.otel.enabled
    {
        bail!("no logging sinks enabled");
    }
    Ok(())
}

#[cfg(feature = "file")]
struct PreparedFile {
    nb: tracing_appender::non_blocking::NonBlocking,
    guard: tracing_appender::non_blocking::WorkerGuard,
    error_counter: tracing_appender::non_blocking::ErrorCounter,
}

struct PreparedParts {
    sinks: SinkGeneration,
    #[cfg(feature = "file")]
    file: Option<PreparedFile>,
}

fn prepare_parts(cfg: &LoggingConfig) -> anyhow::Result<PreparedParts> {
    #[cfg(feature = "file")]
    let (file_nb, file_prep) = if cfg.file.enabled {
        let path = cfg
            .file
            .path
            .as_deref()
            .filter(|p| !p.is_empty())
            .context("logging.file.path")?;
        let path = std::path::Path::new(path);
        let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .context("logging.file.path file name")?;
        if !dir.exists() {
            bail!(
                "logging.file directory does not exist: {} (STARTUP_REJECT_CONFIG)",
                dir.display()
            );
        }
        let probe = dir.join(format!(".exyonq-log-probe-{}", std::process::id()));
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&probe)
        {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
            }
            Err(err) => bail!(
                "logging.file directory not writable: {} ({err}) (STARTUP_REJECT_CONFIG)",
                dir.display()
            ),
        }
        let appender: Box<dyn Write + Send + Sync> = if cfg.file.rotation.enabled {
            Box::new(
                crate::size_rotate::SizeRotatingWriter::open(
                    dir,
                    file_name,
                    cfg.file.rotation.max_bytes,
                    cfg.file.rotation.keep,
                )
                .context("logging.file size rotator")?,
            )
        } else {
            Box::new(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .with_context(|| format!("open logging.file.path {}", path.display()))?,
            )
        };
        let (nb, guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
            .buffered_lines_limit(cfg.queue_capacity.max(1) as usize)
            .finish(appender);
        let error_counter = nb.error_counter();
        (
            Some(nb.clone()),
            Some(PreparedFile {
                nb,
                guard,
                error_counter,
            }),
        )
    } else {
        (None, None)
    };

    #[cfg(feature = "syslog")]
    let syslog = if cfg.syslog.enabled {
        Some(crate::syslog_sink::SyslogLayer::try_new(&cfg.syslog)?)
    } else {
        None
    };

    #[cfg(feature = "journald")]
    let journald_native = if cfg.journald.enabled {
        #[cfg(target_os = "linux")]
        {
            crate::journald_sink::probe()?;
            true
        }
        #[cfg(not(target_os = "linux"))]
        {
            bail!("logging.journald.enabled is Linux-only (STARTUP_REJECT_CONFIG)");
        }
    } else {
        false
    };

    // Journald Layer removed: scrubbed fanout owns native datagram submission
    // so field-level secrets cannot bypass redaction (SEC-CAP061-001 / LA-CAP061-003).

    Ok(PreparedParts {
        sinks: SinkGeneration {
            json: matches!(cfg.format, LogFormat::Json),
            console: cfg.console.enabled.then_some(cfg.console.stream),
            #[cfg(feature = "file")]
            file: file_nb,
            #[cfg(feature = "syslog")]
            syslog,
            #[cfg(feature = "journald")]
            journald_native,
        },
        #[cfg(feature = "file")]
        file: file_prep,
    })
}

fn install_or_reload(cfg: &LoggingConfig) -> anyhow::Result<ObservabilityGuard> {
    if let Err(msg) = exyonq_config_ir::validate_logging(cfg) {
        bail!(msg);
    }
    feature_gates(cfg)?;

    if CONTROLLER.get().is_some() {
        return commit_reload(cfg);
    }

    let gen = 1u64;
    GENERATION.store(gen, Ordering::SeqCst);
    let prepared = prepare_parts(cfg)?;
    let sinks = Arc::new(RwLock::new(prepared.sinks));
    let fanout = FanoutLayer {
        sinks: Arc::clone(&sinks),
    };
    let filter = build_filter(cfg)?;
    let (filter_layer, filter_handle) = reload::Layer::new(filter);

    #[cfg(feature = "otel")]
    let (otel_init, otel_rt) = if cfg.otel.enabled {
        let (layer, rt) = crate::otel::build_otel_layer::<AfterJournald>(&cfg.otel)?;
        (Some(layer), Some(rt))
    } else {
        (None, None)
    };
    #[cfg(feature = "otel")]
    let (otel_layer, otel_handle) = reload::Layer::new(otel_init);

    let registry = Registry::default().with(filter_layer).with(fanout);
    #[cfg(feature = "otel")]
    let registry = registry.with(otel_layer);

    registry
        .try_init()
        .map_err(|e| anyhow::anyhow!("tracing init: {e}"))?;

    #[cfg(feature = "file")]
    let (file_guard, file_error_counter) = match prepared.file {
        Some(p) => (Some(p.guard), Some(p.error_counter)),
        None => (None, None),
    };

    CONTROLLER
        .set(Mutex::new(Controller {
            filter: filter_handle,
            sinks,
            #[cfg(feature = "otel")]
            otel: otel_handle,
            #[cfg(feature = "file")]
            file_guard,
            #[cfg(feature = "file")]
            file_error_counter,
            #[cfg(feature = "otel")]
            otel_rt,
            cfg: cfg.clone(),
        }))
        .map_err(|_| anyhow::anyhow!("observability controller already set"))?;

    Ok(ObservabilityGuard { generation: gen })
}

fn commit_reload(cfg: &LoggingConfig) -> anyhow::Result<ObservabilityGuard> {
    let ctrl_mutex = CONTROLLER
        .get()
        .ok_or_else(|| anyhow::anyhow!("observability controller missing"))?;
    let mut ctrl = ctrl_mutex
        .lock()
        .map_err(|_| anyhow::anyhow!("observability controller poisoned"))?;

    if &ctrl.cfg == cfg {
        return Ok(ObservabilityGuard {
            generation: GENERATION.load(Ordering::SeqCst),
        });
    }

    let gen = GENERATION.load(Ordering::SeqCst).saturating_add(1);
    let prepared = prepare_parts(cfg)?;
    let new_filter = build_filter(cfg)?;

    #[cfg(feature = "otel")]
    let (new_otel_layer, new_otel_rt) = if cfg.otel.enabled {
        let (layer, rt) = crate::otel::build_otel_layer::<AfterJournald>(&cfg.otel)?;
        (Some(layer), Some(rt))
    } else {
        (None, None)
    };

    if let Err(err) = ctrl.filter.reload(new_filter) {
        bail!("observability filter reload failed (KEEP_OLD): {err}");
    }

    // Cap061: OTel reload before sink swap. Journald is part of sink generation
    // (scrubbed fanout) — atomic with sinks write below (LA-CAP061-003).
    // LA-CAP061-005: if OTel fails after filter advanced, restore filter or
    // refuse KEEP_OLD labeling when restore fails (PARTIAL_APPLY).
    #[cfg(feature = "otel")]
    {
        if let Err(err) = ctrl.otel.reload(new_otel_layer) {
            let prior_filter = match build_filter(&ctrl.cfg) {
                Ok(f) => f,
                Err(rebuild_err) => {
                    bail!(
                        "observability otel reload failed and cannot rebuild prior filter (PARTIAL_APPLY): otel={err}; rebuild={rebuild_err}"
                    );
                }
            };
            match ctrl.filter.reload(prior_filter) {
                Ok(()) => {
                    bail!(
                        "{}",
                        otel_fail_after_filter_message(&err.to_string(), Ok(()))
                    )
                }
                Err(restore_err) => {
                    bail!(
                        "{}",
                        otel_fail_after_filter_message(
                            &err.to_string(),
                            Err(restore_err.to_string())
                        )
                    )
                }
            }
        }
        if let Some(old) = ctrl.otel_rt.take() {
            old.shutdown();
        }
        ctrl.otel_rt = new_otel_rt;
    }

    {
        let mut live = ctrl
            .sinks
            .write()
            .map_err(|_| anyhow::anyhow!("sink lock poisoned"))?;
        *live = prepared.sinks;
    }

    #[cfg(feature = "file")]
    {
        if let Some(ref c) = ctrl.file_error_counter {
            crate::bounds::note_file_drops(c.dropped_lines() as u64);
        }
        match prepared.file {
            Some(p) => {
                let _ = p.nb;
                ctrl.file_guard = Some(p.guard);
                ctrl.file_error_counter = Some(p.error_counter);
            }
            None => {
                ctrl.file_guard = None;
                ctrl.file_error_counter = None;
            }
        }
    }

    ctrl.cfg = cfg.clone();
    GENERATION.store(gen, Ordering::SeqCst);
    tracing::info!(
        event = "observability",
        generation = gen,
        "observability generation committed"
    );
    Ok(ObservabilityGuard { generation: gen })
}

pub fn reloadability_matrix() -> &'static [(&'static str, &'static str)] {
    &[
        ("filter/level", "RELOADABLE_VIA_RELOAD_HANDLE"),
        ("format", "RELOADABLE_VIA_SINK_GENERATION_SWAP"),
        ("console selection", "RELOADABLE"),
        ("file path", "RELOADABLE_PREPARE_VALIDATE_COMMIT"),
        ("rotation policy", "RELOADABLE_SIZE_BASED"),
        ("syslog", "RELOADABLE_PREPARE_NEW_CONNECTION"),
        ("journald", "RELOADABLE_VIA_SCRUBBED_FANOUT_NATIVE_DATAGRAM"),
        ("OTLP endpoint", "RELOADABLE_VIA_RELOAD_HANDLE"),
        ("OTLP protocol", "HTTP_PROTOBUF_ONLY_GRPC_REJECT"),
        (
            "redaction policy",
            "EMIT_SITE_SCRUB_PLUS_FANOUT_SCRUB; OTEL_INHERITS_EMIT_CONTRACT",
        ),
        ("access/audit enabled", "CORE_ATOMIC_FLAGS_ON_RELOAD"),
    ]
}

/// Classify OTel-after-filter failure messages (LA-CAP061-005).
/// KEEP_OLD only when prior filter was restored successfully.
#[cfg(any(test, feature = "otel"))]
pub(crate) fn otel_fail_after_filter_message(
    otel_err: &str,
    filter_restore: Result<(), String>,
) -> String {
    match filter_restore {
        Ok(()) => format!("observability otel reload failed (KEEP_OLD): {otel_err}"),
        Err(restore_err) => format!(
            "observability otel reload failed and filter restore failed (PARTIAL_APPLY): otel={otel_err}; filter_restore={restore_err}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::otel_fail_after_filter_message;

    #[test]
    fn otel_fail_keep_old_only_when_filter_restored() {
        let msg = otel_fail_after_filter_message("otel boom", Ok(()));
        assert!(msg.contains("KEEP_OLD"), "{msg}");
        assert!(!msg.contains("PARTIAL_APPLY"), "{msg}");
    }

    #[test]
    fn otel_fail_partial_apply_when_filter_restore_fails() {
        let msg = otel_fail_after_filter_message("otel boom", Err("restore boom".into()));
        assert!(msg.contains("PARTIAL_APPLY"), "{msg}");
        assert!(!msg.contains("(KEEP_OLD)"), "{msg}");
    }
}
