//! Connect timeout classification and adapter mapping (PR5-B1-real-final).

use exyonq_mod_fastcgi::{
    map_client_error, ClientError, MinForwardRequest, PhpFpmClient, WireError, WireTransport,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "linux")]
use std::time::Duration;

static SEQ: AtomicU64 = AtomicU64::new(0);

fn missing_socket_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "exyonq-miss-{}-{}.sock",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn missing_unix_socket_is_502_class_connection_failed() {
    let transport = WireTransport::unix(missing_socket_path(), 1);
    let client = PhpFpmClient::with_transport("php", transport);
    let req = MinForwardRequest::get("/", "/", "/");
    let err = client.forward_min_request(&req).unwrap_err();
    assert!(matches!(
        err,
        ClientError::Wire(WireError::ConnectionFailed)
    ));
    assert_eq!(
        map_client_error(err),
        exyonq_module_api::fcgi_dispatch::FcgiDispatchOutcome::BadGateway
    );
}

#[test]
fn connect_timeout_is_504_class() {
    assert_eq!(
        map_client_error(ClientError::Wire(WireError::Timeout)),
        exyonq_module_api::fcgi_dispatch::FcgiDispatchOutcome::GatewayTimeout
    );
}

#[cfg(target_os = "linux")]
#[test]
fn saturated_backlog_connect_timeout_through_executor() {
    use exyonq_mod_fastcgi::FcgiModuleExecutor;
    use exyonq_module_api::fcgi_dispatch::FcgiBackendExecutor;
    use socket2::{Domain, SockAddr, Socket, Type};

    let dir = std::env::temp_dir().join(format!("exyonq-b1f-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("pool-{}.sock", SEQ.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_file(&path);

    let listener = Socket::new(Domain::UNIX, Type::STREAM, None).expect("socket");
    let addr = SockAddr::unix(&path).expect("addr");
    listener.bind(&addr).expect("bind");
    listener.listen(1).expect("listen");

    let first = exyonq_mod_fastcgi::connect_unix_stream(&path, Duration::from_millis(500))
        .expect("first connect");
    std::mem::forget(first);

    let executor = FcgiModuleExecutor::production_unix(path.clone(), Duration::from_millis(120));
    let request = exyonq_module_api::fcgi_dispatch::FcgiDispatchRequest {
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
    };
    let outcome = executor.dispatch(&request);
    assert_eq!(
        outcome,
        exyonq_module_api::fcgi_dispatch::FcgiDispatchOutcome::GatewayTimeout
    );

    let _ = std::fs::remove_file(&path);
}
