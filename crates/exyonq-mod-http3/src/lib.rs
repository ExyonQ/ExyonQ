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
//! HTTP/3 / QUIC listener runtime (KD4.5) — P13D multi-provider facade.
//!
//! Product compile default (Phase 6R): s2n (`http3-provider-s2n`).
//! Optional: quiche (`http3-provider-quiche`); s2n+quiche dual-compile supported.
//! Quinn legacy (`http3-provider-quinn-legacy`) is rollback-only and exclusive.
//! Selection is startup-only via neutral `[http3].provider` (product: s2n|quiche).

#[cfg(any(
    all(
        feature = "http3-provider-quinn-legacy",
        feature = "http3-provider-s2n"
    ),
    all(
        feature = "http3-provider-quinn-legacy",
        feature = "http3-provider-quiche"
    ),
))]
compile_error!("exyonq-mod-http3: http3-provider-quinn-legacy cannot combine with s2n or quiche");

#[cfg(not(any(
    feature = "http3-provider-quinn-legacy",
    feature = "http3-provider-s2n",
    feature = "http3-provider-quiche",
)))]
compile_error!(
    "exyonq-mod-http3: enable http3-provider-s2n (product default), http3-provider-quiche, and/or http3-provider-quinn-legacy (rollback; exclusive)"
);

use exyonq_mod_tls::TlsSettings;
use exyonq_module_api::http3_runtime::{Http3ConnectionLifecycle, Http3DispatchService};
use std::net::SocketAddr;
use std::sync::Arc;

#[cfg(any(feature = "http3-provider-s2n", feature = "http3-provider-quiche"))]
use exyonq_http3_provider_api::{Http3ProviderConfig, Http3ProviderError, Http3ProviderId};

#[derive(Debug, Clone)]
pub struct Http3Settings {
    pub listen: SocketAddr,
    pub tls: TlsSettings,
    /// Product IR provider (`"s2n"` / `"quiche"`). Ignored for Quinn-only builds.
    /// `None` → resolve via [`resolve_provider`] (IR default s2n when compiled).
    pub provider: Option<String>,
    pub max_ack_delay_ms: u64,
    pub request_body_drain_cap_bytes: usize,
    pub qlog_enabled: bool,
}

impl Http3Settings {
    pub fn legacy(listen: SocketAddr, tls: TlsSettings) -> Self {
        Self {
            listen,
            tls,
            provider: None,
            max_ack_delay_ms: 1,
            request_body_drain_cap_bytes: 64 * 1024,
            qlog_enabled: false,
        }
    }
}

/// Compiled provider ids available in this binary (facade-local registry).
pub fn available_providers() -> Vec<&'static str> {
    [
        #[cfg(feature = "http3-provider-quinn-legacy")]
        "quinn-legacy",
        #[cfg(feature = "http3-provider-s2n")]
        "s2n",
        #[cfg(feature = "http3-provider-quiche")]
        "quiche",
    ]
    .into_iter()
    .collect()
}

#[cfg(not(feature = "http3-provider-quinn-legacy"))]
fn provider_compiled(id: &str) -> bool {
    available_providers().contains(&id)
}

/// Resolve product/legacy provider for this build + settings.
///
/// Missing provider behavior (Phase 5):
/// - Quinn-only build → `quinn-legacy` (public IR provider ignored).
/// - Modern build, provider absent → `"s2n"` if compiled, else the sole modern provider.
/// - Modern build, provider set → must be `s2n`|`quiche` and compiled.
pub fn resolve_provider(settings: &Http3Settings) -> Result<&'static str, String> {
    #[cfg(feature = "http3-provider-quinn-legacy")]
    {
        let _ = settings;
        return Ok("quinn-legacy");
    }

    #[cfg(not(feature = "http3-provider-quinn-legacy"))]
    {
        let requested = match settings.provider.as_deref() {
            None | Some("") => {
                if provider_compiled("s2n") {
                    "s2n"
                } else if provider_compiled("quiche") {
                    "quiche"
                } else {
                    return Err("no http3 provider compiled".into());
                }
            }
            Some(raw) => match raw {
                "s2n" | "s2n-quic" => "s2n",
                "quiche" => "quiche",
                "quinn" | "quinn-legacy" => {
                    return Err(
                        "invalid http3 provider: quinn-legacy is not a public IR option (compile feature only)"
                            .into(),
                    );
                }
                other => return Err(format!("invalid http3 provider: {other}")),
            },
        };
        if !provider_compiled(requested) {
            return Err(format!("http3 provider {requested} not in this build"));
        }
        Ok(requested)
    }
}

#[cfg(feature = "http3-provider-quinn-legacy")]
mod quinn_provider;

/// Serve HTTP/3 with the resolved provider (startup selection only).
pub async fn serve<LC>(
    settings: Http3Settings,
    dispatch: Arc<dyn Http3DispatchService>,
    lifecycle: Arc<LC>,
) -> anyhow::Result<()>
where
    LC: Http3ConnectionLifecycle + Send + Sync + 'static,
{
    let selected = resolve_provider(&settings).map_err(anyhow::Error::msg)?;
    tracing::info!(
        provider = selected,
        available = ?available_providers(),
        max_ack_delay_ms = settings.max_ack_delay_ms,
        "HTTP/3 provider selected"
    );

    match selected {
        "quinn-legacy" => {
            #[cfg(feature = "http3-provider-quinn-legacy")]
            {
                quinn_provider::serve(settings, dispatch, lifecycle).await
            }
            #[cfg(not(feature = "http3-provider-quinn-legacy"))]
            {
                Err(anyhow::Error::msg(
                    "http3 provider quinn-legacy not in this build",
                ))
            }
        }
        "s2n" => {
            #[cfg(feature = "http3-provider-s2n")]
            {
                let config = provider_config(Http3ProviderId::S2n, &settings)?;
                exyonq_http3_provider_s2n::serve(config, dispatch, lifecycle).await
            }
            #[cfg(not(feature = "http3-provider-s2n"))]
            {
                Err(anyhow::Error::msg("http3 provider s2n not in this build"))
            }
        }
        "quiche" => {
            #[cfg(feature = "http3-provider-quiche")]
            {
                let config = provider_config(Http3ProviderId::Quiche, &settings)?;
                exyonq_http3_provider_quiche::serve(config, dispatch, lifecycle).await
            }
            #[cfg(not(feature = "http3-provider-quiche"))]
            {
                Err(anyhow::Error::msg(
                    "http3 provider quiche not in this build",
                ))
            }
        }
        other => Err(anyhow::Error::msg(format!(
            "http3 provider {other} not in this build"
        ))),
    }
}

#[cfg(any(feature = "http3-provider-s2n", feature = "http3-provider-quiche"))]
fn provider_config(
    id: Http3ProviderId,
    settings: &Http3Settings,
) -> anyhow::Result<Http3ProviderConfig> {
    let config = Http3ProviderConfig {
        provider: id,
        listen: settings.listen,
        cert_path: settings.tls.cert_path.clone(),
        key_path: settings.tls.key_path.clone(),
        max_ack_delay_ms: settings.max_ack_delay_ms,
        qlog_enabled: settings.qlog_enabled,
        request_body_drain_cap_bytes: settings.request_body_drain_cap_bytes,
        idle_timeout_ms: 30_000,
        max_concurrent_streams: 256,
        initial_stream_window: 1_000_000,
        initial_connection_window: 10_000_000,
    };
    config
        .validate()
        .map_err(|e: Http3ProviderError| anyhow::Error::msg(e.to_string()))?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use exyonq_module_api::http3_runtime::{
        Http3ConnectionLifecycle, Http3DrainRejected, Http3MaterializedResponse,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct MockLifecycle(Arc<AtomicUsize>);
    struct MockLease(Arc<AtomicUsize>);

    impl Drop for MockLease {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::Release);
        }
    }

    impl Http3ConnectionLifecycle for MockLifecycle {
        type Lease = MockLease;

        fn try_enter_connection(&self) -> Result<MockLease, Http3DrainRejected> {
            self.0.fetch_add(1, Ordering::AcqRel);
            Ok(MockLease(Arc::clone(&self.0)))
        }
    }

    #[test]
    fn materialized_response_roundtrip_fields() {
        let resp = Http3MaterializedResponse {
            status: 404,
            headers: vec![("x-test".to_string(), "1".to_string())],
            body: Bytes::from_static(b"missing"),
        };
        assert_eq!(resp.status, 404);
        assert_eq!(resp.body.len(), 7);
    }

    #[test]
    fn mock_lease_drop_decrements_once() {
        let active = Arc::new(AtomicUsize::new(0));
        let lifecycle = MockLifecycle(Arc::clone(&active));
        let lease = lifecycle.try_enter_connection().unwrap();
        assert_eq!(active.load(Ordering::Acquire), 1);
        drop(lease);
        assert_eq!(active.load(Ordering::Acquire), 0);
    }

    #[test]
    fn registry_lists_compiled_providers() {
        let avail = available_providers();
        assert!(!avail.is_empty());
    }

    #[cfg(all(feature = "http3-provider-s2n", feature = "http3-provider-quiche"))]
    #[test]
    fn dual_resolve_explicit_and_default() {
        let tls = TlsSettings {
            cert_path: "c".into(),
            key_path: "k".into(),
        };
        let mut s = Http3Settings::legacy("127.0.0.1:4433".parse().unwrap(), tls);
        assert_eq!(resolve_provider(&s).unwrap(), "s2n");
        s.provider = Some("quiche".into());
        assert_eq!(resolve_provider(&s).unwrap(), "quiche");
        s.provider = Some("nginx".into());
        assert!(resolve_provider(&s).unwrap_err().contains("invalid"));
        s.provider = Some("quinn-legacy".into());
        assert!(resolve_provider(&s).unwrap_err().contains("not a public"));
    }

    #[cfg(all(
        feature = "http3-provider-s2n",
        not(feature = "http3-provider-quiche"),
        not(feature = "http3-provider-quinn-legacy")
    ))]
    #[test]
    fn s2n_only_rejects_quiche_selection() {
        let tls = TlsSettings {
            cert_path: "c".into(),
            key_path: "k".into(),
        };
        let mut s = Http3Settings::legacy("127.0.0.1:4433".parse().unwrap(), tls);
        s.provider = Some("quiche".into());
        assert!(resolve_provider(&s)
            .unwrap_err()
            .contains("not in this build"));
    }
}
