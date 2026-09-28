//! KD3.4 — wire proxy transport extraction tests.
use exyonq_core::{execute_backend::ProxyDispatchTestGuard, server::state::ServerState, Backend};
use exyonq_mod_proxy::{install_kernel_hooks, ProxyRuntime};
use exyonq_module_api::proxy_wire;
use std::sync::Arc;

async fn state_for(config: exyonq_config_ir::AppConfig) -> Arc<ServerState> {
    let proxy_runtime = Arc::new(ProxyRuntime::new());
    install_kernel_hooks(Arc::clone(&proxy_runtime));
    let _guard: ProxyDispatchTestGuard = ProxyDispatchTestGuard::install(proxy_runtime);
    let proxy_client = exyonq_mod_proxy::build_incoming_client().clone();
    ServerState::new_with_generation(1, config, proxy_client)
        .await
        .expect("server state")
}

#[test]
fn proxy_wire_predicate_matches_api_prefix() {
    let rt = Arc::new(ProxyRuntime::new());
    install_kernel_hooks(rt);
    let head = b"GET /api/health HTTP/1.1\r\nHost: localhost\r\n\r\n";
    assert!(proxy_wire::might_use_proxy_wire(head));
}

#[test]
fn proxy_wire_predicate_rejects_non_api() {
    let rt = Arc::new(ProxyRuntime::new());
    install_kernel_hooks(rt);
    assert!(!proxy_wire::might_use_proxy_wire(
        b"GET /site/x HTTP/1.1\r\n\r\n"
    ));
}

#[tokio::test]
async fn wire_resolve_cluster_via_backend_table() {
    let raw = include_str!("../../tests/fixtures/minimal.toml");
    let config: exyonq_config_ir::AppConfig = raw.parse().unwrap();
    let state = state_for(config).await;
    let (route_idx, _) = state
        .route_index
        .match_route_index_with_host("/api/users", None)
        .expect("api match");
    let via_slot = state
        .snapshot
        .proxy_compiled_slot(0)
        .expect("compiled proxy slot");
    let backend = state.snapshot.resolve_backend(route_idx).expect("backend");
    let Backend::Proxy { cluster_id } = backend else {
        panic!("expected proxy backend");
    };
    assert_eq!(*cluster_id, 0);
    assert_eq!(via_slot.target, "http://127.0.0.1:9000");
    assert_eq!(state.snapshot.proxy_cluster_for_route(route_idx), Some(0));
}

#[test]
fn kd3_5_proxy_cache_owned_by_module_not_core_shim() {
    assert!(
        !std::path::Path::new("src/proxy/mod.rs").exists(),
        "core proxy shim removed in KD3.5"
    );
    assert!(
        !std::path::Path::new("src/proxy_cache.rs").exists(),
        "core proxy_cache removed in KD3.5"
    );
    let handler = std::fs::read_to_string("src/server/handler.rs").expect("handler");
    assert!(handler.contains("load_get_for_cache"));
    assert!(!handler.contains("proxy::forward_get_streaming"));
    let module_cache = std::fs::read_to_string("../crates/exyonq-mod-proxy/src/proxy_cache.rs")
        .expect("mod cache");
    assert!(module_cache.contains("prepare_proxy_cache_load"));
}

#[test]
fn wire_io_header_end_detects_crlf() {
    let buf = b"GET / HTTP/1.1\r\nHost: x\r\n\r\ntail";
    let end = exyonq_mod_proxy::wire_io::find_header_end(buf).expect("headers");
    assert_eq!(&buf[end + 4..], b"tail");
}

#[test]
fn handler_uses_module_websocket_not_forward_request_shim() {
    let src = std::fs::read_to_string("src/server/handler.rs").expect("handler");
    assert!(src.contains("forward_websocket"));
    assert!(!src.contains("forward_request"));
}

#[test]
fn wire_dispatch_delegates_to_proxy_wire_contract() {
    let src = std::fs::read_to_string("src/server/wire_dispatch.rs").expect("wire_dispatch");
    assert!(src.contains("proxy_wire::serve_proxy_wire"));
    assert!(!src.contains("forward_get("));
    assert!(!src.contains("strip_hop_by_hop"));
}
