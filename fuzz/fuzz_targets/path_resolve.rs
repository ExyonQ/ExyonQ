#![no_main]

//! P15-WS2-PATH: path normalization / static root containment.
//! Invariants: NO_PANIC, NO_OUT_OF_ROOT_PATH (errors only), DETERMINISTIC.

use libfuzzer_sys::fuzz_target;
use std::path::Path;
use std::sync::Once;

static INIT: Once = Once::new();

fn ensure_root() {
    INIT.call_once(|| {
        let root = Path::new("/tmp/exyonq-fuzz-root");
        let _ = std::fs::create_dir_all(root);
        let _ = std::fs::write(root.join("index.html"), b"ok");
    });
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() || data.len() > 4096 {
        return;
    }
    ensure_root();
    let request_path = std::str::from_utf8(data).unwrap_or("/site/x");
    let _ = exyonq_mod_static::resolve_path(
        Path::new("/tmp/exyonq-fuzz-root"),
        "/site",
        request_path,
        Some("index.html"),
    );
});
