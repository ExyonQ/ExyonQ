//! Plan 05B limits — fail closed.

pub const MAX_DIRECTORY_DEPTH: usize = 32;
pub const MAX_HTACCESS_FILES_PER_VHOST: usize = 4096;
pub const MAX_BYTES_PER_FILE: usize = 1024 * 1024;
pub const MAX_BYTES_TOTAL_PER_VHOST: usize = 16 * 1024 * 1024;
pub const MAX_DIRECTIVES_PER_FILE: usize = 8192;
pub const WATCHER_DEBOUNCE_MS: u64 = 150;
