//! OpenTelemetry OTLP tracing (Cap061). Metrics remain Cap054 Prometheus.
//!
//! # Redaction / OTel (SEC-CAP061-OTLP-REDACTION)
//!
//! `OpenTelemetryLayer` observes tracing fields independently of Fanout scrub.
//! Cap061 product contract: **never attach cleartext sensitive values to
//! spans/events** — apply `scrub_field_blob` / `header_value_for_log` at emit
//! time (see `emit_redaction_probe`). Access/audit paths must not log Authorization,
//! Cookie, or equivalent cleartext fields.

use anyhow::{bail, Context};
use exyonq_config_ir::{OtelLoggingConfig, OtlpProtocol};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use tracing::Subscriber;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::registry::LookupSpan;

pub struct OtelRuntime {
    provider: SdkTracerProvider,
}

impl OtelRuntime {
    /// Force-flush pending spans (bounded by SDK export timeout). Used before
    /// generation swap so E2E/collector observation is not timing-luck.
    pub fn force_flush(&self) {
        if let Err(err) = self.provider.force_flush() {
            tracing::warn!(
                event = "observability",
                error = %err,
                "otel provider force_flush failed"
            );
        }
    }

    pub fn shutdown(self) {
        self.force_flush();
        // Best-effort final shutdown; errors are diagnostic only.
        if let Err(err) = self.provider.shutdown() {
            tracing::warn!(
                event = "observability",
                error = %err,
                "otel provider shutdown failed"
            );
        }
    }
}

/// `opentelemetry-otlp` programmatic `with_endpoint` does **not** append the
/// signal path (unlike `OTEL_EXPORTER_OTLP_ENDPOINT` env). Cap061 product config
/// documents a collector base URL (`http://host:4318`); normalize to the OTLP
/// HTTP traces path before building the exporter.
pub(crate) fn normalize_otlp_http_traces_endpoint(endpoint: &str) -> String {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if trimmed.ends_with("/v1/traces") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1/traces")
    }
}

pub fn build_otel_layer<S>(
    cfg: &OtelLoggingConfig,
) -> anyhow::Result<(
    OpenTelemetryLayer<S, opentelemetry_sdk::trace::Tracer>,
    OtelRuntime,
)>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    if cfg.metrics_export {
        bail!(
            "logging.otel.metrics_export is forbidden; Cap054 Prometheus remains metrics authority"
        );
    }
    let endpoint = cfg
        .endpoint
        .as_deref()
        .filter(|s| !s.is_empty())
        .context("logging.otel.endpoint")?;
    let endpoint = normalize_otlp_http_traces_endpoint(endpoint);

    let protocol = match cfg.protocol {
        OtlpProtocol::HttpProtobuf => Protocol::HttpBinary,
        OtlpProtocol::Grpc => {
            bail!("logging.otel.protocol=grpc not enabled in this build; use http_protobuf (OTLP/HTTP)")
        }
    };

    let exporter = SpanExporter::builder()
        .with_http()
        .with_protocol(protocol)
        .with_endpoint(endpoint)
        .build()
        .context("build OTLP HTTP span exporter")?;

    let resource = Resource::builder()
        .with_service_name(cfg.service_name.clone())
        .build();

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build();

    let tracer = provider.tracer("exyonq");
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);
    Ok((layer, OtelRuntime { provider }))
}

#[cfg(test)]
mod tests {
    use super::normalize_otlp_http_traces_endpoint;

    #[test]
    fn appends_v1_traces_to_base_url() {
        assert_eq!(
            normalize_otlp_http_traces_endpoint("http://127.0.0.1:4318"),
            "http://127.0.0.1:4318/v1/traces"
        );
        assert_eq!(
            normalize_otlp_http_traces_endpoint("http://127.0.0.1:4318/"),
            "http://127.0.0.1:4318/v1/traces"
        );
        assert_eq!(
            normalize_otlp_http_traces_endpoint("http://127.0.0.1:4318/v1/traces"),
            "http://127.0.0.1:4318/v1/traces"
        );
    }
}
