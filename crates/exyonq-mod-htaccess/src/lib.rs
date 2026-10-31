/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
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
