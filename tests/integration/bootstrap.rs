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
//! Minimal composition-root hooks for integration tests (mirrors CLI serve bootstrap).

use exyonq_mod_proxy::HyperClientConfig;
use std::sync::{Arc, Once};
use std::time::Duration;

static INTEGRATION_BOOTSTRAP: Once = Once::new();

/// Register dispatch services and control plane once per test process.
pub fn ensure_integration_modules() {
    INTEGRATION_BOOTSTRAP.call_once(|| {
        // H3/proxy GET·HEAD·POST use process-wide OnceLock Hyper clients
        // (`get_empty_body_client` / `post_body_client`). Default connect_timeout
        // (500ms) flakes under parallel integration suites → 502. Raise before
        // any client is constructed, then eagerly init all three pools.
        exyonq_mod_proxy::set_hyper_client_config(HyperClientConfig {
            tcp_nodelay: true,
            tcp_keepalive: Duration::from_secs(60),
            connect_timeout: Duration::from_secs(2),
            pool_idle_timeout: Duration::from_secs(90),
            pool_max_idle_per_host: 64,
        });
        let _ = exyonq_mod_proxy::build_incoming_client();
        let _ = exyonq_mod_proxy::get_empty_body_client();
        // `post_body_client` is crate-private; first POST uses CONFIG set above.

        let proxy_reg = exyonq_mod_proxy::registration_with_default_runtime();
        let _ = exyonq_core::register_proxy_dispatch_service(proxy_reg.service);

        let static_runtime = Arc::new(exyonq_mod_static::StaticRuntime::new());
        let static_service: Arc<dyn exyonq_core::StaticDispatchService> = static_runtime.clone();
        let _ = exyonq_core::register_static_dispatch_service(static_service);
        exyonq_mod_static::install_kernel_hooks(static_runtime);

        let _ = exyonq_ops_runtime::register_control_plane();
        let _ = exyonq_discovery_runtime::register_discovery_runtime();
        let _ = exyonq_acme::register_acme_integration();
        let _ = exyonq_metrics::register_observability_runtime();
        let _ = exyonq_reload_runtime::register_reload_runtime();
    });
}
