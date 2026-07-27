use std::fs;
use std::path::PathBuf;

fn src_rs_files() -> Vec<PathBuf> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    fs::read_dir(src)
        .expect("src readable")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "rs"))
        .collect()
}

#[test]
fn src_has_no_transport_symbols_outside_wire_module() {
    for path in src_rs_files() {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if matches!(
            name,
            "wire.rs"
                | "unix_transport.rs"
                | "unix_connect.rs"
                | "fcgi_stream.rs"
                | "conn_pool.rs"
                | "pooled_forward.rs"
                | "adapter.rs" // PoolEndpoint address parsing (no live connect)
        ) {
            continue;
        }
        let content = fs::read_to_string(&path).expect("read src");
        // P15-WS1-FCGI-002: precise tokens — avoid false positives on disconnect()/reconnect().
        let forbidden_checks: &[(&str, &dyn Fn(&str) -> bool)] = &[
            ("TcpStream", &|c| c.contains("TcpStream")),
            ("UnixStream", &|c| c.contains("UnixStream")),
            ("tokio::net", &|c| c.contains("tokio::net")),
            ("std::net", &|c| c.contains("std::net")),
            ("unsafe", &|c| c.contains("unsafe")),
            ("connect(", &|c| {
                c.contains(".connect(") || c.contains("::connect(") || c.contains("fn connect(")
            }),
        ];
        for (label, pred) in forbidden_checks {
            assert!(
                !pred(&content),
                "{} must not reference `{label}`",
                path.display()
            );
        }
    }
}

#[test]
fn wire_module_is_isolated() {
    let wire = fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/wire.rs"))
        .expect("wire.rs");
    assert!(wire.contains("UnixStream"));
    assert!(wire.contains("TcpStream"));
}

#[test]
fn parser_transport_and_client_modules_present() {
    let lib = include_str!("../src/lib.rs");
    assert!(lib.contains("mod parser"));
    assert!(lib.contains("mod record"));
    assert!(lib.contains("mod transport"));
    assert!(lib.contains("mod client"));
    assert!(lib.contains("mod encode"));
    assert!(lib.contains("mod mock"));
    assert!(lib.contains("mod wire"));
    assert!(lib.contains("mod unix_transport"));
    assert!(lib.contains("mod unix_connect"));
    assert!(lib.contains("mod pool"));
    assert!(lib.contains("mod caps"));
    assert!(lib.contains("PR3A_PHASE"));
    assert!(lib.contains("PR4A_PHASE"));
    assert!(lib.contains("PR5A_MIN_PHASE"));
}
