//! Internal safety limits for NGINX import (not user-configurable yet).

pub const MAX_INCLUDE_DEPTH: usize = 16;
pub const MAX_INCLUDED_FILES: usize = 1024;
pub const MAX_TOTAL_INPUT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TOKEN_LENGTH: usize = 16 * 1024;
pub const MAX_DIRECTIVES: usize = 100_000;
pub const MAX_BLOCK_DEPTH: usize = 64;
pub const MAX_TOKENS: usize = 500_000;
pub const MAX_UPSTREAM_BLOCKS: usize = 4096;
pub const MAX_SERVERS_PER_UPSTREAM: usize = 1024;
pub const MAX_GENERATED_ENDPOINTS: usize = 16384;
pub const MAX_LOCATIONS_PER_SERVER: usize = 8192;
pub const MAX_TRY_FILES_CANDIDATES: usize = 64;
pub const MAX_REWRITE_RULES_PER_LOCATION: usize = 256;
pub const MAX_REGEX_LENGTH: usize = 4096;
pub const MAX_VARIABLE_REFERENCES_PER_DIRECTIVE: usize = 128;
