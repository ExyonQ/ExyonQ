#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 65536 {
        return;
    }
    let input = std::str::from_utf8(data).unwrap_or("");
    let _ = exyonq_core::AppConfig::parse_str(input);
});
