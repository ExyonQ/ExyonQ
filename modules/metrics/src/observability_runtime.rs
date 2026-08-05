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
//! Composition-root observability registration (KD4.12).

use crate::{bench_trace, kernel_shell_metrics, KernelShellMetrics};
use exyonq_module_api::kernel_observation::{
    register_kernel_observation_service, KernelObservationRegisterError,
};
use exyonq_module_api::observability_runtime::{
    ensure_runtime_prometheus_hook, register_prometheus_appender,
};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservabilityRegisterError {
    KernelObservationAlreadyRegistered,
}

impl From<KernelObservationRegisterError> for ObservabilityRegisterError {
    fn from(value: KernelObservationRegisterError) -> Self {
        match value {
            KernelObservationRegisterError::AlreadyRegistered => {
                Self::KernelObservationAlreadyRegistered
            }
            KernelObservationRegisterError::Poisoned => Self::KernelObservationAlreadyRegistered,
        }
    }
}

/// Register kernel observation + cold-path Prometheus append hooks (once per process).
pub fn register_observability_runtime() -> Result<(), ObservabilityRegisterError> {
    register_kernel_observation_service(Arc::new(KernelShellMetrics))?;

    register_prometheus_appender(kernel_shell_metrics::append_prometheus);
    register_prometheus_appender(bench_trace::append_prometheus);

    #[cfg(target_os = "linux")]
    register_prometheus_appender(exyonq_module_api::static_epoll::append_prometheus);

    register_prometheus_appender(exyonq_mod_htaccess::append_htaccess_prometheus);

    ensure_runtime_prometheus_hook(crate::register_runtime_prometheus_append);
    Ok(())
}
