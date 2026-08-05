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
//! KD1 closure — FastCGI 501 observability contract (core shell vs module runtime).

use exyonq_core::{
    execute_backend, Backend, FcgiDispatchTestGuard, FcgiRuntimeRegistration,
    DEFAULT_FCGI_MAX_CONCURRENCY,
};
use exyonq_metrics::{fastcgi_http_501_total, fcgi_responses_501_total, KernelShellMetrics};
use exyonq_mod_fastcgi::{fcgi_metrics_snapshot, FcgiRuntime, MockFcgiExecutor};
use exyonq_module_api::fcgi_dispatch::{
    FcgiBackendExecutor, FcgiDispatchOutcome, FcgiDispatchRequest,
};
use exyonq_module_api::kernel_observation::KernelObservationTestGuard;
use std::sync::Arc;

/// Serializes global HTTP 501 shell counters across observability scenarios.
static METRICS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_fcgi_request() -> FcgiDispatchRequest {
    FcgiDispatchRequest {
        pool_id: 0,
        method: "GET".into(),
        request_uri: "/index.php".into(),
        query_string: String::new(),
        script_name: "/index.php".into(),
        script_filename: "/var/www/index.php".into(),
        path_info: None,
        document_root: "/var/www".into(),
        server_name: "localhost".into(),
        server_port: 80,
        remote_addr: "127.0.0.1".into(),
        server_protocol: "HTTP/1.1".into(),
        content_type: None,
        body: Vec::new(),
        headers: Vec::new(),
    }
}

struct NotRegisteredExecutor;

impl FcgiBackendExecutor for NotRegisteredExecutor {
    fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
        FcgiDispatchOutcome::NotRegistered
    }
}

fn install_fcgi_runtime(executor: Arc<dyn FcgiBackendExecutor>) -> FcgiDispatchTestGuard {
    let runtime = Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor,
            pool_capacities: vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)],
        })
        .expect("fcgi runtime"),
    );
    FcgiDispatchTestGuard::install(runtime)
}

#[tokio::test]
async fn unregistered_module_increments_core_http_501_only() {
    let _lock = METRICS_TEST_LOCK.lock().await;
    let _obs = KernelObservationTestGuard::install(Arc::new(KernelShellMetrics));
    let _guard = FcgiDispatchTestGuard::force_absent();
    let http_before = fastcgi_http_501_total();
    let runtime_before = fcgi_metrics_snapshot().responses_501;
    let aggregate_before = fcgi_responses_501_total();

    let outcome = execute_backend(
        &Backend::Fastcgi { pool_id: 0 },
        Some(test_fcgi_request()),
        None,
        None,
    )
    .await
    .expect("fastcgi path");
    assert_eq!(outcome.status, 501);

    assert_eq!(fastcgi_http_501_total(), http_before + 1);
    assert_eq!(fcgi_metrics_snapshot().responses_501, runtime_before);
    assert_eq!(fcgi_responses_501_total(), aggregate_before + 1);
}

#[tokio::test]
async fn registered_runtime_not_registered_increments_module_only() {
    let _lock = METRICS_TEST_LOCK.lock().await;
    let _obs = KernelObservationTestGuard::install(Arc::new(KernelShellMetrics));
    let _guard = install_fcgi_runtime(Arc::new(NotRegisteredExecutor));

    let http_before = fastcgi_http_501_total();
    let runtime_before = fcgi_metrics_snapshot().responses_501;
    let aggregate_before = fcgi_responses_501_total();

    let outcome = execute_backend(
        &Backend::Fastcgi { pool_id: 0 },
        Some(test_fcgi_request()),
        None,
        None,
    )
    .await
    .expect("fastcgi path");
    assert_eq!(outcome.status, 501);

    assert_eq!(fastcgi_http_501_total(), http_before);
    assert_eq!(fcgi_metrics_snapshot().responses_501, runtime_before + 1);
    assert_eq!(fcgi_responses_501_total(), aggregate_before + 1);
}

#[tokio::test]
async fn registered_success_does_not_increment_501_counters() {
    let _lock = METRICS_TEST_LOCK.lock().await;
    let _obs = KernelObservationTestGuard::install(Arc::new(KernelShellMetrics));
    let _guard = install_fcgi_runtime(Arc::new(MockFcgiExecutor::pr5b1_default()));

    let http_before = fastcgi_http_501_total();
    let runtime_before = fcgi_metrics_snapshot().responses_501;
    let aggregate_before = fcgi_responses_501_total();

    let outcome = execute_backend(
        &Backend::Fastcgi { pool_id: 0 },
        Some(test_fcgi_request()),
        None,
        None,
    )
    .await
    .expect("fastcgi path");
    assert_eq!(outcome.status, 200);

    assert_eq!(fastcgi_http_501_total(), http_before);
    assert_eq!(fcgi_metrics_snapshot().responses_501, runtime_before);
    assert_eq!(fcgi_responses_501_total(), aggregate_before);
}
