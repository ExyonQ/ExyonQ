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

#[cfg(test)]
mod tests {
    use crate::kernel_shell_metrics::{
        append_prometheus as append_kernel_shell_prometheus, fastcgi_http_501_total,
        KernelShellMetrics,
    };
    use crate::observability_runtime::{
        register_observability_runtime, ObservabilityRegisterError,
    };
    use exyonq_module_api::kernel_observation::{
        clear_kernel_observation_for_tests, kernel_observation_registration_test_gate,
        note_fastcgi_http_501, KernelObservationTestGuard,
    };
    use exyonq_module_api::observability_runtime::begin_prometheus_appender_test;
    use std::sync::Arc;

    #[test]
    fn kernel_shell_metrics_increment_and_export() {
        let _gate = kernel_observation_registration_test_gate();
        clear_kernel_observation_for_tests();
        let _guard = KernelObservationTestGuard::install(Arc::new(KernelShellMetrics));
        let before = fastcgi_http_501_total();
        note_fastcgi_http_501();
        assert_eq!(fastcgi_http_501_total(), before + 1);
        let mut out = String::new();
        append_kernel_shell_prometheus(&mut out);
        assert!(out.contains("exyonq_kernel_fastcgi_http_501_total"));
    }

    #[test]
    fn register_observability_runtime_is_register_once() {
        let _gate = kernel_observation_registration_test_gate();
        // Serialize against overlay-mutating tests (do not call clear_* while
        // another thread may hold begin_prometheus_appender_test).
        let _overlay = begin_prometheus_appender_test();
        clear_kernel_observation_for_tests();
        register_observability_runtime().expect("first");
        assert_eq!(
            register_observability_runtime(),
            Err(ObservabilityRegisterError::KernelObservationAlreadyRegistered)
        );
        clear_kernel_observation_for_tests();
        // Overlay cleared on `_overlay` Drop — no ungated clear.
    }
}
