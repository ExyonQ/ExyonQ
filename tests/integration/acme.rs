//! ACME HTTP-01 integration tests (challenge path vs redirect).

use exyonq_acme::{http01_key_authorization, AcmeIntegrationRuntime};
use exyonq_core::config::{
    AppConfig, RedirectConfig, RouteConfig, RouteMatch, ServerConfig, ServerNames, UpstreamConfig,
};
use exyonq_core::server::handler::{serve_http3_request, ConnectionContext};
use exyonq_core::server::state::ServerState;
use exyonq_mod_proxy::build_incoming_client;
use exyonq_module_api::{AcmeIntegrationTestGuard, ACME_HTTP01_WELL_KNOWN_PREFIX};
use hyper::header::HeaderValue;
use hyper::{Method, Request};
use std::collections::HashMap;
use std::sync::Arc;

fn redirect_config() -> AppConfig {
    AppConfig {
        config_version: 1,
        includes: Vec::new(),
        servers: vec![ServerConfig {
            listen: "127.0.0.1:8080".into(),
            server_name: ServerNames::None,
            routes: vec!["root".into()],
            tls: None,
            http3_listen: None,
        }],
        routes: vec![RouteConfig {
            name: "root".into(),
            r#match: RouteMatch {
                path: "/".into(),
                host: None,
            },
            upstream: None,
            root: None,
            index: None,
            redirect: Some(RedirectConfig {
                status: 301,
                location: "https://example.com/".into(),
            }),
            rewrite: None,
            fastcgi: None,
            htaccess: Default::default(),
            cache: None,
        }],
        upstreams: HashMap::from([(
            "backend".into(),
            UpstreamConfig {
                name: "backend".into(),
                target: "http://127.0.0.1:9000".into(),
                timeout_ms: 5000,
            },
        )]),
        pools_fcgi: HashMap::new(),
        cache_policies: HashMap::new(),
        modules: Default::default(),
        static_section: Default::default(),
        full_page_cache: Default::default(),
        http3: Default::default(),
    }
}

/// HTTP-01 must resolve while a site-wide redirect is configured (handler checks ACME before routing).
#[tokio::test]
async fn acme_renew_with_http_redirect_active() {
    let runtime = Arc::new(AcmeIntegrationRuntime::new());
    runtime
        .challenge_store()
        .set("renew-token", "key-auth-body");
    let _guard = AcmeIntegrationTestGuard::install(runtime.clone());

    let path = format!("{ACME_HTTP01_WELL_KNOWN_PREFIX}renew-token");
    let body =
        http01_key_authorization(&runtime, &path).expect("registered token serves challenge");
    assert_eq!(body, "key-auth-body");

    let config = redirect_config();
    let proxy_client = build_incoming_client();
    assert!(ServerState::new(config, proxy_client).await.is_ok());
}

#[tokio::test]
async fn acme_unknown_token_falls_through_to_redirect() {
    let runtime = Arc::new(AcmeIntegrationRuntime::new());
    let _guard = AcmeIntegrationTestGuard::install(runtime);
    let config = redirect_config();
    let proxy_client = build_incoming_client();
    let state = ServerState::new(config, proxy_client.clone())
        .await
        .unwrap();

    let req = Request::builder()
        .method(Method::GET)
        .uri("http://127.0.0.1/")
        .body(())
        .unwrap();

    let response = serve_http3_request(
        ConnectionContext {
            state,
            proxy_client,
            x_forwarded_for: HeaderValue::from_static("127.0.0.1"),
            ops: exyonq_core::lifecycle::LifecycleState::new(),
        },
        req,
    )
    .await;

    assert_eq!(response.status(), 301);
    assert_eq!(
        response.headers().get("location").unwrap(),
        "https://example.com/"
    );
}
