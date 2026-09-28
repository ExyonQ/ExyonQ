//! Cap061 — `[logging]` observability IR (RD-005: implement intended product surface).

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn default_format() -> LogFormat {
    LogFormat::Text
}

fn default_level() -> String {
    "info".into()
}

fn default_console_stream() -> ConsoleStream {
    ConsoleStream::Stdout
}

fn default_file_max_bytes() -> u64 {
    100 * 1024 * 1024
}

fn default_file_keep() -> u32 {
    7
}

fn default_syslog_facility() -> String {
    "daemon".into()
}

fn default_otel_protocol() -> OtlpProtocol {
    OtlpProtocol::HttpProtobuf
}

fn default_otel_service_name() -> String {
    "exyonq".into()
}

fn default_queue_capacity() -> u32 {
    8192
}

/// Top-level `[logging]` — production observability configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingConfig {
    /// EnvFilter-compatible filter / level directive (e.g. `info`, `exyonq=debug`).
    #[serde(default = "default_level")]
    pub level: String,
    #[serde(default = "default_format")]
    pub format: LogFormat,
    #[serde(default)]
    pub console: ConsoleLoggingConfig,
    #[serde(default)]
    pub file: FileLoggingConfig,
    #[serde(default)]
    pub syslog: SyslogLoggingConfig,
    #[serde(default)]
    pub journald: JournaldLoggingConfig,
    #[serde(default)]
    pub otel: OtelLoggingConfig,
    #[serde(default)]
    pub access: AccessLoggingConfig,
    #[serde(default)]
    pub audit: AuditLoggingConfig,
    /// Bounded async worker queue capacity for non-blocking sinks.
    #[serde(default = "default_queue_capacity")]
    pub queue_capacity: u32,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_level(),
            format: default_format(),
            console: ConsoleLoggingConfig::default(),
            file: FileLoggingConfig::default(),
            syslog: SyslogLoggingConfig::default(),
            journald: JournaldLoggingConfig::default(),
            otel: OtelLoggingConfig::default(),
            access: AccessLoggingConfig::default(),
            audit: AuditLoggingConfig::default(),
            queue_capacity: default_queue_capacity(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConsoleStream {
    Stdout,
    Stderr,
    /// Route INFO/access to stdout and WARN+/errors to stderr.
    Split,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleLoggingConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_console_stream")]
    pub stream: ConsoleStream,
}

impl Default for ConsoleLoggingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            stream: default_console_stream(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileLoggingConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub rotation: FileRotationConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRotationConfig {
    /// ExyonQ-managed size rotation (strategy A). External logrotate + reopen is B.
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_file_max_bytes")]
    pub max_bytes: u64,
    #[serde(default = "default_file_keep")]
    pub keep: u32,
}

impl Default for FileRotationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_bytes: default_file_max_bytes(),
            keep: default_file_keep(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyslogLoggingConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    /// `unix` | `udp` | `tcp`
    #[serde(default)]
    pub transport: Option<String>,
    /// Unix socket path or `host:port` for udp/tcp.
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default = "default_syslog_facility")]
    pub facility: String,
}

impl Default for SyslogLoggingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            transport: None,
            address: None,
            facility: default_syslog_facility(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct JournaldLoggingConfig {
    /// Native journald structured submission (not merely systemd stdout capture).
    #[serde(default = "default_false")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OtlpProtocol {
    HttpProtobuf,
    Grpc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OtelLoggingConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default = "default_otel_protocol")]
    pub protocol: OtlpProtocol,
    #[serde(default = "default_otel_service_name")]
    pub service_name: String,
    /// Cap054 remains Prometheus metrics authority; OTel metrics off by default.
    #[serde(default = "default_false")]
    pub metrics_export: bool,
}

impl Default for OtelLoggingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: None,
            protocol: default_otel_protocol(),
            service_name: default_otel_service_name(),
            metrics_export: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessLoggingConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for AccessLoggingConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditLoggingConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for AuditLoggingConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Validate `[logging]` after deserialize.
pub fn validate_logging(cfg: &LoggingConfig) -> Result<(), String> {
    if cfg.queue_capacity == 0 || cfg.queue_capacity > 1_048_576 {
        return Err("logging.queue_capacity must be 1..=1048576".into());
    }
    if cfg.file.enabled {
        match cfg.file.path.as_deref() {
            None | Some("") => {
                return Err("logging.file.path required when logging.file.enabled".into());
            }
            Some(path) if path.contains('\0') => {
                return Err("logging.file.path must not contain NUL".into());
            }
            _ => {}
        }
        if cfg.file.rotation.enabled {
            if cfg.file.rotation.max_bytes == 0 {
                return Err("logging.file.rotation.max_bytes must be non-zero when enabled".into());
            }
            if cfg.file.rotation.keep == 0 {
                return Err("logging.file.rotation.keep must be non-zero when enabled".into());
            }
        }
    }
    if cfg.syslog.enabled {
        let transport = cfg
            .syslog
            .transport
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(transport.as_str(), "unix" | "udp" | "tcp") {
            return Err("logging.syslog.transport must be unix|udp|tcp when enabled".into());
        }
        match cfg.syslog.address.as_deref() {
            None | Some("") => {
                return Err("logging.syslog.address required when logging.syslog.enabled".into());
            }
            _ => {}
        }
    }
    if cfg.otel.enabled {
        match cfg.otel.endpoint.as_deref() {
            None | Some("") => {
                return Err("logging.otel.endpoint required when logging.otel.enabled".into());
            }
            Some(ep) if !(ep.starts_with("http://") || ep.starts_with("https://")) => {
                return Err("logging.otel.endpoint must be http(s)://…".into());
            }
            _ => {}
        }
        if cfg.otel.service_name.trim().is_empty() {
            return Err("logging.otel.service_name must be non-empty when otel enabled".into());
        }
    }
    // Cap054 sole metrics authority — reject even when otel.enabled=false.
    if cfg.otel.metrics_export {
        return Err(
            "logging.otel.metrics_export is forbidden; Cap054 Prometheus is the sole metrics authority"
                .into(),
        );
    }
    let level = cfg.level.trim();
    if level.is_empty() {
        return Err("logging.level must be non-empty".into());
    }
    Ok(())
}
