//! Plan 05B — `.htaccess` overlay v0 (offline compile + atomic publish).

mod ast;
mod compiler;
mod discovery;
mod limits;
mod parser;
mod report;
mod runtime;
mod store;
mod watcher;

pub use ast::{DirectiveSupport, ParsedDirective, ParsedFile};
pub use compiler::{compile_from_discovered, compile_vhost_overlay, CompileError, CompileOutput};
pub use discovery::{discover_htaccess_files, DiscoveredFile, DiscoveryError};
pub use limits::*;
pub use parser::{parse_htaccess, ParseError};
pub use report::{CompileReport, SourceLoc};
pub use runtime::{
    append_htaccess_prometheus, htaccess_directory_index_hits_total,
    htaccess_directory_index_misses_total, htaccess_front_controller_bypass_directory_total,
    htaccess_front_controller_bypass_file_total, htaccess_front_controller_rewrites_total,
    htaccess_internal_rewrite_loop_total, htaccess_overlay_merge_total,
    htaccess_overlay_redirect_total, HtaccessRuntime,
};
pub use store::{OverlayMetrics, OverlayPublisher};
pub use watcher::{compile_and_publish, initial_compile, spawn_watcher, HtaccessSite};
