#![no_main]

//! P15-WS2-PROXY: hop-by-hop / CL+TE / duplicate CL safety.
//! Invariants: NO_PANIC, PROXY_HEADER_SANITIZATION, NO_FAIL_OPEN.
//! Uses owner-side harness helper (no direct `http` dep in fuzz leaf).

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 16_384 {
        return;
    }
    exyonq_mod_proxy::fuzz_exercise_proxy_headers(data);
});
