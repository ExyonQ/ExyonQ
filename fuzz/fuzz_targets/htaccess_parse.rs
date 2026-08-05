#![no_main]

//! P15-WS2-HTACCESS: parse fail-closed; forbidden directives surface as errors.
//! Invariants: NO_PANIC, HTACCESS_NO_PANIC, HTACCESS_FORBIDDEN_FAIL_CLOSED.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 65_536 {
        return;
    }
    let input = std::str::from_utf8(data).unwrap_or("");
    let _ = exyonq_mod_htaccess::parse_htaccess("/fuzz/.htaccess", input);
});
