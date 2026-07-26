//! KD4.2 — htaccess overlay runtime contract (policy in module, probes injected).

use std::sync::Arc;

/// Register-once errors for composition-root htaccess runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HtaccessRegisterError {
    AlreadyRegistered,
    Poisoned,
}

/// Resource kind from a neutral filesystem probe (no `std::path` in contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HtaccessResourceKind {
    File,
    Directory,
    Missing,
}

/// Backend context for overlay evaluation on a matched structural route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HtaccessRouteBackend {
    Static {
        root_slot: u32,
    },
    Fastcgi {
        pool_id: u32,
        document_root: String,
    },
    /// Structural route without static/fcgi backend (proxy, redirect-only, etc.).
    Passthrough,
}

/// Input for one overlay evaluation pass on a request path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtaccessRouteEvaluationRequest {
    pub site_id: String,
    pub request_path: String,
    /// Path component used to rebuild the client URI after directory-index rewrite.
    pub uri_path: String,
    pub backend: HtaccessRouteBackend,
}

/// Outcome of overlay runtime evaluation — handler applies mechanically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HtaccessRouteEvaluationOutcome {
    /// No overlay published for site (should not occur when site_id is set).
    NoOverlay,
    /// External redirect from compiled overlay rules.
    Redirect { status: u16, location: String },
    /// Serve an on-disk file (FastCGI-route static fallback).
    ServeStaticFile { filesystem_path: String },
    /// Terminal: no directory index match and no front-controller (module-owned decision).
    NotFound,
    /// Continue backend dispatch with optional internal rewrites.
    Continue {
        request_path: String,
        uri_path: String,
        fcgi_script_uri: Option<String>,
        /// Overlay front-controller failure (500 loop, 502 missing target).
        overlay_http_error: Option<u16>,
    },
}

/// Neutral filesystem/static probes executed by core; module owns policy decisions.
pub trait HtaccessRuntimeProbes: Send + Sync {
    fn probe_static_directory_index(
        &self,
        root_slot: u32,
        dir_uri: &str,
        candidates: &[String],
    ) -> Option<String>;

    fn probe_fcgi_directory_index_script(
        &self,
        document_root: &str,
        dir_uri: &str,
        candidate_uri: &str,
    ) -> bool;

    fn probe_fcgi_directory_index_static_file(
        &self,
        document_root: &str,
        child_uri: &str,
    ) -> Option<String>;

    #[allow(clippy::result_unit_err)]
    fn probe_requested_resource(
        &self,
        document_root: &str,
        uri_path: &str,
    ) -> Result<HtaccessResourceKind, ()>;

    fn fcgi_script_resolvable(&self, document_root: &str, uri_path: &str) -> bool;
}

/// Overlay lookup source — typically backed by `OverlayPublisher` in mod-htaccess.
pub trait HtaccessOverlaySource: Send + Sync {
    fn overlay_for_site(&self, site_id: &str) -> Option<Arc<crate::VhostOverlay>>;
}

/// Runtime overlay policy service (module-owned implementation).
pub trait HtaccessRuntimeService: Send + Sync {
    fn evaluate_route(
        &self,
        request: &HtaccessRouteEvaluationRequest,
    ) -> HtaccessRouteEvaluationOutcome;

    fn htaccess_overlay_merge_total(&self) -> u64;
    fn htaccess_overlay_redirect_total(&self) -> u64;
    fn htaccess_directory_index_hits_total(&self) -> u64;
    fn htaccess_directory_index_misses_total(&self) -> u64;
    fn htaccess_front_controller_rewrites_total(&self) -> u64;
    fn htaccess_front_controller_bypass_file_total(&self) -> u64;
    fn htaccess_front_controller_bypass_directory_total(&self) -> u64;
    fn htaccess_internal_rewrite_loop_total(&self) -> u64;
}
