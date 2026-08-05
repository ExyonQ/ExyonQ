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
//! Plan 09 Tier 1.1 — `fcgi_pool.document_root` propagation and dispatch precedence.

use std::sync::Arc;

use exyonq_config_ir::FcgiPoolConfig;
use exyonq_core::{
    build_fcgi_dispatch_request, compile_fcgi_pool_slots, compile_runtime_plan, AppConfig, Backend,
    FcgiBackendExecutor, FcgiDispatchOutcome, FcgiDispatchRequest,
};
use exyonq_core::{FcgiRuntimeRegistration, DEFAULT_FCGI_MAX_CONCURRENCY};
use exyonq_module_api::fcgi_script_resolver::{
    FastcgiScriptResolutionOutcome, FastcgiScriptResolverTestGuard,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

fn dual_pool_config(root_a: &Path, root_b: &Path) -> AppConfig {
    let raw = format!(
        r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["route_a", "route_b"]

[[route]]
name = "route_a"
match = {{ path = "/a/index.php" }}
fastcgi = "site-a"

[[route]]
name = "route_b"
match = {{ path = "/b/index.php" }}
fastcgi = "site-b"

[[fcgi_pool]]
name = "site-b"
address = "unix:/run/b.sock"
document_root = "{}"
max_concurrency = 8

[[fcgi_pool]]
name = "site-a"
address = "unix:/run/a.sock"
document_root = "{}"
max_concurrency = 16
"#,
        root_b.display(),
        root_a.display()
    );
    raw.parse().expect("dual pool config")
}

#[test]
fn compile_fcgi_pool_slots_preserves_per_pool_document_roots() {
    let mut pools = HashMap::new();
    pools.insert(
        "site-b".into(),
        FcgiPoolConfig {
            name: "site-b".into(),
            address: "unix:/run/b.sock".into(),
            document_root: Some(PathBuf::from("/srv/b")),
            max_concurrency: 8,
            max_connections: None,
            transport: "unix".to_string(),
            idle_timeout_ms: 30_000,
            total_timeout_ms: 30_000,
            checkout_timeout_ms: 5_000,
        },
    );
    pools.insert(
        "site-a".into(),
        FcgiPoolConfig {
            name: "site-a".into(),
            address: "unix:/run/a.sock".into(),
            document_root: Some(PathBuf::from("/srv/a")),
            max_concurrency: 16,
            max_connections: None,
            transport: "unix".to_string(),
            idle_timeout_ms: 30_000,
            total_timeout_ms: 30_000,
            checkout_timeout_ms: 5_000,
        },
    );
    let slots = compile_fcgi_pool_slots(&pools);
    assert_eq!(slots.len(), 2);
    assert_eq!(slots[0].name, "site-a");
    assert_eq!(
        slots[0]
            .document_root
            .as_ref()
            .map(|p| p.display().to_string()),
        Some("/srv/a".into())
    );
    assert_eq!(slots[1].name, "site-b");
    assert_eq!(
        slots[1]
            .document_root
            .as_ref()
            .map(|p| p.display().to_string()),
        Some("/srv/b".into())
    );
}

#[test]
fn snapshot_maps_pool_id_to_correct_document_root() {
    let dir_a = tempfile::tempdir().expect("tempdir a");
    let dir_b = tempfile::tempdir().expect("tempdir b");
    let config = dual_pool_config(dir_a.path(), dir_b.path());
    let snap = compile_runtime_plan(1, config).expect("compile plan");
    assert_eq!(
        snap.fcgi_pool_document_root(0)
            .map(|p| p.display().to_string()),
        Some(dir_a.path().display().to_string())
    );
    assert_eq!(
        snap.fcgi_pool_document_root(1)
            .map(|p| p.display().to_string()),
        Some(dir_b.path().display().to_string())
    );
}

#[test]
fn pool_without_document_root_compiles_as_none() {
    let mut pools = HashMap::new();
    pools.insert(
        "php".into(),
        FcgiPoolConfig {
            name: "php".into(),
            address: "unix:/run/php.sock".into(),
            document_root: None,
            max_concurrency: 16,
            max_connections: None,
            transport: "unix".to_string(),
            idle_timeout_ms: 30_000,
            total_timeout_ms: 30_000,
            checkout_timeout_ms: 5_000,
        },
    );
    let slots = compile_fcgi_pool_slots(&pools);
    assert!(slots[0].document_root.is_none());
}

#[test]
fn build_dispatch_request_uses_pool_a_script_under_root_a() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("index.php"), b"<?php").expect("write");
    let root = dir.path().to_path_buf();
    let _resolver =
        FastcgiScriptResolverTestGuard::install(exyonq_mod_fastcgi::FastcgiScriptResolver::arc());
    let request = build_fcgi_dispatch_request(
        0,
        Some(&root),
        "GET",
        "/index.php",
        "",
        "/index.php",
        Vec::new(),
        Vec::new(),
        "127.0.0.1",
        80,
    )
    .expect("dispatch request");
    assert!(request.script_filename.ends_with("index.php"));
    assert_eq!(
        request.document_root,
        root.canonicalize()
            .expect("canonical root")
            .to_string_lossy()
    );
}

#[test]
fn build_dispatch_request_preserves_path_info() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("api.php"), b"<?php").expect("write");
    let root = dir.path().to_path_buf();
    let _resolver =
        FastcgiScriptResolverTestGuard::install(exyonq_mod_fastcgi::FastcgiScriptResolver::arc());
    let request = build_fcgi_dispatch_request(
        0,
        Some(&root),
        "GET",
        "/api.php/posts/hello",
        "",
        "/api.php/posts/hello",
        Vec::new(),
        Vec::new(),
        "127.0.0.1",
        80,
    )
    .expect("dispatch request");
    assert!(request.script_filename.ends_with("api.php"));
    assert_eq!(request.path_info.as_deref(), Some("/posts/hello"));
}

#[test]
fn build_dispatch_request_pool_b_differs_from_pool_a() {
    let dir_a = tempfile::tempdir().expect("tempdir a");
    let dir_b = tempfile::tempdir().expect("tempdir b");
    std::fs::write(dir_a.path().join("a.php"), b"<?php").expect("write a");
    std::fs::write(dir_b.path().join("b.php"), b"<?php").expect("write b");
    let root_a = dir_a.path().to_path_buf();
    let root_b = dir_b.path().to_path_buf();
    let _resolver =
        FastcgiScriptResolverTestGuard::install(exyonq_mod_fastcgi::FastcgiScriptResolver::arc());
    let req_a = build_fcgi_dispatch_request(
        0,
        Some(&root_a),
        "GET",
        "/a.php",
        "",
        "/a.php",
        Vec::new(),
        Vec::new(),
        "127.0.0.1",
        80,
    )
    .expect("req a");
    let req_b = build_fcgi_dispatch_request(
        1,
        Some(&root_b),
        "GET",
        "/b.php",
        "",
        "/b.php",
        Vec::new(),
        Vec::new(),
        "127.0.0.1",
        80,
    )
    .expect("req b");
    assert!(req_a.script_filename.contains("a.php"));
    assert!(req_b.script_filename.contains("b.php"));
    assert_ne!(req_a.document_root, req_b.document_root);
}

#[test]
fn missing_document_root_fails_before_dispatch() {
    std::env::remove_var("EXYONQ_FCGI_DOCUMENT_ROOT");
    let _resolver = FastcgiScriptResolverTestGuard::install(Arc::new(
        exyonq_mod_fastcgi::FastcgiScriptResolver::new(),
    ));
    let err = build_fcgi_dispatch_request(
        0,
        None,
        "GET",
        "/index.php",
        "",
        "/index.php",
        Vec::new(),
        Vec::new(),
        "127.0.0.1",
        80,
    )
    .unwrap_err();
    assert_eq!(err, FastcgiScriptResolutionOutcome::Forbidden);
}

#[tokio::test]
async fn script_resolution_failure_does_not_invoke_executor() {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    struct CountingExecutor;
    impl FcgiBackendExecutor for CountingExecutor {
        fn dispatch(&self, _request: &FcgiDispatchRequest) -> FcgiDispatchOutcome {
            CALLS.fetch_add(1, Ordering::SeqCst);
            FcgiDispatchOutcome::BadGateway
        }
    }
    let _fcgi_guard = exyonq_core::FcgiDispatchTestGuard::install(Arc::new(
        exyonq_mod_fastcgi::FcgiRuntime::new(FcgiRuntimeRegistration {
            executor: Arc::new(CountingExecutor),
            pool_capacities: vec![(0, DEFAULT_FCGI_MAX_CONCURRENCY)],
        })
        .expect("runtime"),
    ));

    std::env::remove_var("EXYONQ_FCGI_DOCUMENT_ROOT");
    let _resolver = FastcgiScriptResolverTestGuard::install(Arc::new(
        exyonq_mod_fastcgi::FastcgiScriptResolver::new(),
    ));
    let err = build_fcgi_dispatch_request(
        0,
        None,
        "GET",
        "/index.php",
        "",
        "/index.php",
        Vec::new(),
        Vec::new(),
        "127.0.0.1",
        80,
    )
    .unwrap_err();
    assert_eq!(err, FastcgiScriptResolutionOutcome::Forbidden);
    assert_eq!(CALLS.load(Ordering::SeqCst), 0);
}

#[test]
fn nginx_import_toml_document_root_reaches_runtime_slots() {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/nginx/tier1-fastcgi-unix.conf");
    let input = std::fs::read_to_string(&fixture).expect("read fixture");
    let (toml, _) = exyonq_compat_nginx::migrate(&input).expect("migrate");
    let config: AppConfig = toml.parse().expect("parse migrated toml");
    let pool = config.pools_fcgi.get("srv1_loc1_fcgi").expect("pool");
    assert_eq!(
        pool.document_root.as_ref().map(|p| p.display().to_string()),
        Some("/srv/php".into())
    );
    let snap = compile_runtime_plan(1, config).expect("compile");
    let backend = snap.resolve_backend(0).expect("fastcgi backend");
    let pool_id = match backend {
        Backend::Fastcgi { pool_id } => *pool_id,
        _ => panic!("expected fastcgi backend"),
    };
    assert_eq!(
        snap.fcgi_pool_document_root(pool_id)
            .map(|p| p.display().to_string()),
        Some("/srv/php".into())
    );
}
