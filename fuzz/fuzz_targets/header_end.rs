#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 16_384 {
        return;
    }
    let _ = exyonq_core::find_header_end(data);
    // When a complete header block is present, exercise framing validation
    // (duplicate CL, TE+CL, obs-fold, etc.) without panicking.
    if let Some(end) = exyonq_core::find_header_end(data) {
        let head_len = end + 4;
        if head_len <= data.len() {
            let _ = exyonq_core::request_headers_safe_for_wire(&data[..head_len]);
        }
    }
});
