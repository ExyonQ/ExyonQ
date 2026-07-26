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
//! Plan 08 PR5-B1 — live FastCGI 200 via production registration + mock roundtrip.

use exyonq_core::{
    execute_backend, Backend, FcgiDispatchTestGuard, FcgiRuntimeRegistration,
    DEFAULT_FCGI_MAX_CONCURRENCY,
};
use exyonq_metrics::{fcgi_responses_501_total, KernelShellMetrics};
use exyonq_mod_fastcgi::fcgi_responses_200_total;
use exyonq_mod_fastcgi::{
    parse_cgi_stdout, FcgiRuntime, MockFcgiExecutor, PR5B1_BODY, PR5B1_STDOUT,
};
use exyonq_module_api::fcgi_dispatch::{
    FcgiBackendExecutor, FcgiDispatchOutcome, FcgiDispatchRequest,
};
use exyonq_module_api::kernel_observation::KernelObservationTestGuard;
use std::sync::Arc;

fn install_mock_executor(executor: Arc<dyn FcgiBackendExecutor>) -> FcgiDispatchTestGuard {
    let runtime = Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor,
            pool_capacities: vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)],
        })
        .expect("fcgi runtime"),
    );
    FcgiDispatchTestGuard::install(runtime)
}

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

#[tokio::test]
async fn pr5_b1_no_executor_returns_501() {
    let _obs = KernelObservationTestGuard::install(Arc::new(KernelShellMetrics));
    let _guard = FcgiDispatchTestGuard::force_absent();
    let before = fcgi_responses_501_total();
    let outcome = execute_backend(
        &Backend::Fastcgi { pool_id: 0 },
        Some(test_fcgi_request()),
        None,
        None,
    )
    .await
    .expect("fastcgi path");
    assert_eq!(outcome.status, 501);
    assert_ne!(outcome.status, 503);
    assert_eq!(fcgi_responses_501_total(), before + 1);
}

#[tokio::test]
async fn pr5_b1_registered_success_returns_200_with_exact_body() {
    let _guard = install_mock_executor(Arc::new(MockFcgiExecutor::pr5b1_default()));
    let before = fcgi_responses_200_total();
    let outcome = execute_backend(
        &Backend::Fastcgi { pool_id: 0 },
        Some(test_fcgi_request()),
        None,
        None,
    )
    .await
    .expect("fastcgi path");
    assert_eq!(outcome.status, 200);
    assert_eq!(outcome.body, PR5B1_BODY);
    assert!(outcome
        .headers
        .iter()
        .any(|(k, v)| k == "content-type" && v == "text/plain"));
    assert!(outcome
        .headers
        .iter()
        .any(|(k, v)| k == "content-length" && v == "19"));
    assert_eq!(fcgi_responses_200_total(), before + 1);
}

#[tokio::test]
async fn pr5_b1_mock_roundtrip_byte_exact() {
    let executor = MockFcgiExecutor::pr5b1_default();
    let outcome = executor.dispatch(&test_fcgi_request());
    match outcome {
        FcgiDispatchOutcome::Success(response) => {
            assert_eq!(response.status, 200);
            assert_eq!(response.body, PR5B1_BODY);
            let parsed = parse_cgi_stdout(PR5B1_STDOUT).expect("stdout parse");
            assert_eq!(parsed.body, response.body);
        }
        other => panic!("expected success, got {other:?}"),
    }
}

struct FailureExecutor(FcgiDispatchOutcome);

impl FcgiBackendExecutor for FailureExecutor {
    fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
        self.0.clone()
    }
}

#[tokio::test]
async fn pr5_b1_failure_and_timeout_mappings() {
    {
        let _guard =
            install_mock_executor(Arc::new(FailureExecutor(FcgiDispatchOutcome::BadGateway)));
        let outcome = execute_backend(
            &Backend::Fastcgi { pool_id: 0 },
            Some(test_fcgi_request()),
            None,
            None,
        )
        .await
        .expect("fastcgi");
        assert_eq!(outcome.status, 502);
    }

    {
        let _guard = install_mock_executor(Arc::new(FailureExecutor(
            FcgiDispatchOutcome::GatewayTimeout,
        )));
        let outcome = execute_backend(
            &Backend::Fastcgi { pool_id: 0 },
            Some(test_fcgi_request()),
            None,
            None,
        )
        .await
        .expect("fastcgi");
        assert_eq!(outcome.status, 504);
    }
}

#[test]
fn pr5_b1_core_has_no_direct_mod_fastcgi_dependency() {
    let manifest = include_str!("../Cargo.toml");
    let production = manifest
        .split("[dev-dependencies]")
        .next()
        .expect("manifest");
    assert!(
        !production.contains("exyonq-mod-fastcgi"),
        "core production deps must not depend on exyonq-mod-fastcgi"
    );
    let handler = include_str!("../src/server/handler.rs");
    assert!(
        !handler.contains("exyonq_mod_fastcgi::"),
        "handler must use fcgi_cache_registry, not direct mod-fastcgi imports"
    );
    assert!(
        handler.contains("fcgi_cache_registry::"),
        "handler must route FastCGI cache through fcgi_cache_registry"
    );
    let lib = include_str!("../src/lib.rs");
    assert!(
        !lib.contains("exyonq_mod_fastcgi::FcgiRuntime"),
        "core lib must not wire FastCGI runtime (cache serve imports only)"
    );
}

#[test]
fn pr5_b1_delegate_timeout_is_outer_barrier() {
    use exyonq_mod_fastcgi::{FCGI_CONNECT_TIMEOUT, FCGI_DELEGATE_TIMEOUT};
    assert_eq!(FCGI_DELEGATE_TIMEOUT.as_secs(), 30);
    assert!(FCGI_DELEGATE_TIMEOUT > FCGI_CONNECT_TIMEOUT);
}

#[test]
fn pr5_b1_wire_dispatch_untouched() {
    let wire_dispatch = include_str!("../src/server/wire_dispatch.rs");
    assert!(
        !wire_dispatch.contains("FcgiBackendExecutor"),
        "wire_dispatch must remain unwired for PR5-B1"
    );
}
