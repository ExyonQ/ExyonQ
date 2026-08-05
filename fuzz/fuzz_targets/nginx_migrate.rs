#![no_main]

//! P15-WS2-NGINX: offline importer — no panic, fail-closed on hostile input.
//! Invariants: NO_PANIC, NGINX_IMPORT_NO_PANIC, NO_INVALID_IR_ACCEPTED (via report errors).

use exyonq_compat_nginx::{migrate_source, MigrateOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 65_536 {
        return;
    }
    let input = std::str::from_utf8(data).unwrap_or("");
    let opts = MigrateOptions {
        dry_run: true,
        strict: true,
        ..MigrateOptions::default()
    };
    let _ = migrate_source("fuzz.conf", input, &opts);
});
