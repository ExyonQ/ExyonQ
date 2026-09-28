//! Syslog sink via real syslog crate (unix/udp/tcp).

use anyhow::{bail, Context};
use exyonq_config_ir::SyslogLoggingConfig;
use syslog::{Facility, Formatter3164};
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::{format::Writer, FormatFields};
use tracing_subscriber::layer::Context as LayerContext;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

type SysLogger = syslog::Logger<syslog::LoggerBackend, Formatter3164>;

pub struct SyslogLayer {
    logger: std::sync::Mutex<SysLogger>,
}

impl SyslogLayer {
    pub fn try_new(cfg: &SyslogLoggingConfig) -> anyhow::Result<Self> {
        let transport = cfg.transport.as_deref().unwrap_or("").to_ascii_lowercase();
        let address = cfg
            .address
            .as_deref()
            .filter(|s| !s.is_empty())
            .context("logging.syslog.address")?;
        let facility = parse_facility(&cfg.facility)?;
        let formatter = Formatter3164 {
            facility,
            hostname: None,
            process: "exyonq".into(),
            pid: std::process::id(),
        };
        let logger = match transport.as_str() {
            "unix" => syslog::unix_custom(formatter, address)
                .with_context(|| format!("syslog unix {address}"))?,
            "udp" => {
                let local = "0.0.0.0:0";
                syslog::udp(formatter, local, address)
                    .with_context(|| format!("syslog udp {address}"))?
            }
            "tcp" => {
                syslog::tcp(formatter, address).with_context(|| format!("syslog tcp {address}"))?
            }
            other => bail!("unsupported syslog transport `{other}`"),
        };
        Ok(Self {
            logger: std::sync::Mutex::new(logger),
        })
    }

    /// Emit a pre-formatted line (used by Cap061 fan-out sink generation).
    pub fn emit_formatted(&self, level: &tracing::Level, line: &str) {
        let Ok(mut logger) = self.logger.lock() else {
            crate::bounds::note_sink_error();
            return;
        };
        let result = match *level {
            tracing::Level::ERROR => logger.err(line),
            tracing::Level::WARN => logger.warning(line),
            tracing::Level::INFO => logger.info(line),
            tracing::Level::DEBUG => logger.debug(line),
            tracing::Level::TRACE => logger.debug(line),
        };
        if result.is_err() {
            crate::bounds::note_sink_error();
        }
    }
}

fn parse_facility(raw: &str) -> anyhow::Result<Facility> {
    Ok(match raw.trim().to_ascii_lowercase().as_str() {
        "kern" => Facility::LOG_KERN,
        "user" => Facility::LOG_USER,
        "mail" => Facility::LOG_MAIL,
        "daemon" => Facility::LOG_DAEMON,
        "auth" => Facility::LOG_AUTH,
        "syslog" => Facility::LOG_SYSLOG,
        "lpr" => Facility::LOG_LPR,
        "news" => Facility::LOG_NEWS,
        "uucp" => Facility::LOG_UUCP,
        "cron" => Facility::LOG_CRON,
        "authpriv" => Facility::LOG_AUTHPRIV,
        "ftp" => Facility::LOG_FTP,
        "local0" => Facility::LOG_LOCAL0,
        "local1" => Facility::LOG_LOCAL1,
        "local2" => Facility::LOG_LOCAL2,
        "local3" => Facility::LOG_LOCAL3,
        "local4" => Facility::LOG_LOCAL4,
        "local5" => Facility::LOG_LOCAL5,
        "local6" => Facility::LOG_LOCAL6,
        "local7" => Facility::LOG_LOCAL7,
        other => bail!("unknown logging.syslog.facility `{other}`"),
    })
}

impl<S> Layer<S> for SyslogLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: LayerContext<'_, S>) {
        let mut buf = String::new();
        let field_format = tracing_subscriber::fmt::format::DefaultFields::new();
        let _ = field_format.format_fields(Writer::new(&mut buf), event);
        let meta = event.metadata();
        let scrubbed = crate::redaction::scrub_field_blob(&buf);
        let line = format!("{} {}: {}", meta.level(), meta.target(), scrubbed);
        self.emit_formatted(meta.level(), &line);
    }
}
