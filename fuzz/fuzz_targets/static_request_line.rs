#![no_main]

//! P15-WS2-HTTP1: request-line + header framing via owned wire helpers.
//! Invariants: NO_PANIC, HTTP1_FRAMING_FAIL_CLOSED.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 8192 {
        return;
    }
    let _ = exyonq_core::find_header_end(data);
    if let Some(end) = exyonq_core::find_header_end(data) {
        let head_len = end + 4;
        if head_len <= data.len() {
            let _ = exyonq_core::request_headers_safe_for_wire(&data[..head_len]);
        }
    }
    // First-line UTF-8 probe (method/target/version shape) without accepting invalid state.
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = text.lines().next().map(|line| {
            let mut parts = line.split_whitespace();
            let _method = parts.next();
            let _target = parts.next();
            let _version = parts.next();
        });
    }
});
