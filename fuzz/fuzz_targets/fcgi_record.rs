#![no_main]

use exyonq_mod_fastcgi::record::MAX_RECORD_FRAME_LEN;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_RECORD_FRAME_LEN {
        return;
    }
    let _ = exyonq_mod_fastcgi::parse_record(data);
});
