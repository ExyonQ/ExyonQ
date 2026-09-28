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
//! Plan 08 PR5-B2 — bounded FastCGI concurrency, live 503, per-pool isolation.

use exyonq_core::{
    clear_global_fcgi_dispatch_for_register_once_test, contract_service_registration_test_gate,
    execute_backend, fcgi_metrics_assert_guard, register_fcgi_dispatch_service, Backend,
    FcgiDispatchTestGuard,
};
use exyonq_mod_fastcgi::{
    fcgi_responses_503_total, fcgi_saturation_rejections_total, FcgiRuntime, ScriptedFcgiExecutor,
};
use exyonq_module_api::fcgi_dispatch::{
    FcgiBackendExecutor, FcgiDispatchOutcome, FcgiDispatchRequest, FcgiRuntimeRegistration,
    FcgiSuccessResponse,
};
use std::sync::Arc;
use std::time::Duration;

fn install_runtime(registration: FcgiRuntimeRegistration) -> FcgiDispatchTestGuard {
    let runtime = Arc::new(FcgiRuntime::new(registration).expect("fcgi runtime"));
    FcgiDispatchTestGuard::install(runtime)
}

fn request_for_pool(pool_id: u32) -> FcgiDispatchRequest {
    FcgiDispatchRequest {
        pool_id,
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

fn success_200() -> FcgiDispatchOutcome {
    FcgiDispatchOutcome::Success(FcgiSuccessResponse {
        status: 200,
        headers: vec![("content-type".into(), "text/plain".into())],
        body: b"ok".to_vec(),
    })
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn pr5_b2_connect_failure_stays_502_not_503() {
    let _metrics_gate = fcgi_metrics_assert_guard().await;
    struct Fail502;
    impl FcgiBackendExecutor for Fail502 {
        fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
            FcgiDispatchOutcome::BadGateway
        }
    }
    let _guard = install_runtime(FcgiRuntimeRegistration {
        executor: Arc::new(Fail502),
        pool_capacities: vec![(0, 1)],
    });
    let before = fcgi_responses_503_total();
    let outcome = execute_backend(
        &Backend::Fastcgi { pool_id: 0 },
        Some(request_for_pool(0)),
        None,
        None,
    )
    .await
    .expect("fastcgi");
    assert_eq!(outcome.status, 502);
    assert_eq!(fcgi_responses_503_total(), before);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn pr5_b2_timeout_stays_504_not_503() {
    let _metrics_gate = fcgi_metrics_assert_guard().await;
    struct Fail504;
    impl FcgiBackendExecutor for Fail504 {
        fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
            FcgiDispatchOutcome::GatewayTimeout
        }
    }
    let _guard = install_runtime(FcgiRuntimeRegistration {
        executor: Arc::new(Fail504),
        pool_capacities: vec![(0, 16)],
    });
    let before = fcgi_responses_503_total();
    let outcome = execute_backend(
        &Backend::Fastcgi { pool_id: 0 },
        Some(request_for_pool(0)),
        None,
        None,
    )
    .await
    .expect("fastcgi");
    assert_eq!(outcome.status, 504);
    assert_eq!(fcgi_responses_503_total(), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::await_holding_lock)]
async fn pr5_b2_dual_pool_saturation_is_isolated() {
    let _metrics_gate = fcgi_metrics_assert_guard().await;
    let _gate = contract_service_registration_test_gate();
    clear_global_fcgi_dispatch_for_register_once_test();

    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);

    struct PoolAwareGate {
        started_tx: std::sync::mpsc::Sender<(u32,)>,
        release_rx: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl FcgiBackendExecutor for PoolAwareGate {
        fn dispatch(&self, request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
            let _ = self.started_tx.send((request.pool_id,));
            if request.pool_id == 0 {
                if let Ok(rx) = self.release_rx.lock() {
                    let _ = rx.recv();
                }
            }
            success_200()
        }
    }

    register_fcgi_dispatch_service(Arc::new(
        FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(PoolAwareGate {
                started_tx,
                release_rx: std::sync::Mutex::new(release_rx),
            }),
            pool_capacities: vec![(0, 1), (1, 16)],
        })
        .expect("fcgi runtime"),
    ))
    .expect("register global fcgi for multi-thread test");

    let before_sat = fcgi_saturation_rejections_total();

    let fut_a = tokio::spawn(async move {
        execute_backend(
            &Backend::Fastcgi { pool_id: 0 },
            Some(request_for_pool(0)),
            None,
            None,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok((pool,)) = started_rx.try_recv() {
                assert_eq!(pool, 0);
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("pool A started");

    let outcome_b = execute_backend(
        &Backend::Fastcgi { pool_id: 0 },
        Some(request_for_pool(0)),
        None,
        None,
    )
    .await
    .expect("pool 0 saturated");
    assert_eq!(outcome_b.status, 503);

    let outcome_other = execute_backend(
        &Backend::Fastcgi { pool_id: 1 },
        Some(request_for_pool(1)),
        None,
        None,
    )
    .await
    .expect("pool 1 still available");
    assert_eq!(outcome_other.status, 200);

    release_tx.send(()).expect("release pool 0");
    let outcome_a = fut_a.await.expect("join A").expect("pool 0 done");
    assert_eq!(outcome_a.status, 200);
    assert_eq!(fcgi_saturation_rejections_total(), before_sat + 1);
}

#[tokio::test]
async fn pr5_b2_scripted_success_still_200_with_capacity() {
    let _guard = install_runtime(FcgiRuntimeRegistration {
        executor: Arc::new(ScriptedFcgiExecutor::pr5b1_default()),
        pool_capacities: vec![(0, 16)],
    });
    let outcome = execute_backend(
        &Backend::Fastcgi { pool_id: 0 },
        Some(request_for_pool(0)),
        None,
        None,
    )
    .await
    .expect("fastcgi");
    assert_eq!(outcome.status, 200);
}

#[test]
fn pr5_b2_wire_dispatch_untouched() {
    let wire_dispatch = include_str!("../src/server/wire_dispatch.rs");
    assert!(
        !wire_dispatch.contains("ServiceUnavailable"),
        "wire_dispatch must remain unwired for PR5-B2"
    );
}
