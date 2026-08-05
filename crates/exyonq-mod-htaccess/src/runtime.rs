//! KD4.2 — overlay runtime evaluation (policy owner).

use exyonq_module_api::htaccess_runtime::{
    HtaccessOverlaySource, HtaccessResourceKind, HtaccessRouteBackend,
    HtaccessRouteEvaluationOutcome, HtaccessRouteEvaluationRequest, HtaccessRuntimeProbes,
    HtaccessRuntimeService,
};
use exyonq_module_api::{
    join_directory_index_uri, lookup_overlay, validate_directory_index_candidate,
    CompiledFrontController, OverlayLookupResult, VhostOverlay, MAX_DIRECTORY_INDEX_CANDIDATES,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static OVERLAY_REDIRECT_TOTAL: AtomicU64 = AtomicU64::new(0);
static OVERLAY_MERGE_TOTAL: AtomicU64 = AtomicU64::new(0);
static DIRECTORY_INDEX_HITS: AtomicU64 = AtomicU64::new(0);
static DIRECTORY_INDEX_MISSES: AtomicU64 = AtomicU64::new(0);
static FRONT_CONTROLLER_REWRITES: AtomicU64 = AtomicU64::new(0);
static FRONT_CONTROLLER_BYPASS_FILE: AtomicU64 = AtomicU64::new(0);
static FRONT_CONTROLLER_BYPASS_DIRECTORY: AtomicU64 = AtomicU64::new(0);
static INTERNAL_REWRITE_LOOP: AtomicU64 = AtomicU64::new(0);

pub struct HtaccessRuntime {
    overlay_source: Arc<dyn HtaccessOverlaySource>,
    probes: Arc<dyn HtaccessRuntimeProbes>,
}

impl HtaccessRuntime {
    pub fn new(
        overlay_source: Arc<dyn HtaccessOverlaySource>,
        probes: Arc<dyn HtaccessRuntimeProbes>,
    ) -> Self {
        Self {
            overlay_source,
            probes,
        }
    }
}

impl HtaccessOverlaySource for crate::store::OverlayPublisher {
    fn overlay_for_site(&self, site_id: &str) -> Option<Arc<VhostOverlay>> {
        self.get(site_id)
    }
}

impl HtaccessRuntimeService for HtaccessRuntime {
    fn evaluate_route(
        &self,
        request: &HtaccessRouteEvaluationRequest,
    ) -> HtaccessRouteEvaluationOutcome {
        let Some(overlay) = self.overlay_source.overlay_for_site(&request.site_id) else {
            return HtaccessRouteEvaluationOutcome::NoOverlay;
        };

        OVERLAY_MERGE_TOTAL.fetch_add(1, Ordering::Relaxed);
        let (directory_index, front_controller) =
            match lookup_overlay(&request.request_path, &overlay) {
                OverlayLookupResult::Redirect { status, location } => {
                    OVERLAY_REDIRECT_TOTAL.fetch_add(1, Ordering::Relaxed);
                    return HtaccessRouteEvaluationOutcome::Redirect { status, location };
                }
                OverlayLookupResult::Miss => {
                    return HtaccessRouteEvaluationOutcome::Continue {
                        request_path: request.request_path.clone(),
                        uri_path: request.uri_path.clone(),
                        fcgi_script_uri: None,
                        overlay_http_error: None,
                    };
                }
                OverlayLookupResult::Continue {
                    directory_index,
                    indexes_disabled: _,
                    front_controller,
                } => (directory_index, front_controller),
            };

        let mut request_path = request.request_path.clone();
        let mut uri_path = request.uri_path.clone();
        let mut fcgi_script_uri = None;
        let mut serve_static: Option<String> = None;
        let mut directory_index_not_found = false;

        if request.request_path.ends_with('/') {
            if let Some(candidates) = directory_index {
                if !candidates.is_empty() {
                    match &request.backend {
                        HtaccessRouteBackend::Static { root_slot } => {
                            if let Some(resolved) = self.probes.probe_static_directory_index(
                                *root_slot,
                                &request.request_path,
                                candidates.as_ref(),
                            ) {
                                DIRECTORY_INDEX_HITS.fetch_add(1, Ordering::Relaxed);
                                request_path = resolved.clone();
                                uri_path = resolved;
                            } else {
                                DIRECTORY_INDEX_MISSES.fetch_add(1, Ordering::Relaxed);
                                directory_index_not_found = true;
                            }
                        }
                        HtaccessRouteBackend::Fastcgi { document_root, .. } => {
                            match resolve_fcgi_directory_index(
                                self.probes.as_ref(),
                                document_root,
                                &request.request_path,
                                candidates.as_ref(),
                            ) {
                                FcgiDirIndex::Script(uri) => {
                                    DIRECTORY_INDEX_HITS.fetch_add(1, Ordering::Relaxed);
                                    request_path = uri.clone();
                                    fcgi_script_uri = Some(uri);
                                }
                                FcgiDirIndex::StaticFile {
                                    uri,
                                    filesystem_path,
                                } => {
                                    DIRECTORY_INDEX_HITS.fetch_add(1, Ordering::Relaxed);
                                    request_path = uri.clone();
                                    uri_path = uri;
                                    serve_static = Some(filesystem_path);
                                }
                                FcgiDirIndex::Miss => {
                                    DIRECTORY_INDEX_MISSES.fetch_add(1, Ordering::Relaxed);
                                    directory_index_not_found = true;
                                }
                            }
                        }
                        HtaccessRouteBackend::Passthrough => {}
                    }
                }
            }
        }

        if let Some(path) = serve_static {
            return HtaccessRouteEvaluationOutcome::ServeStaticFile {
                filesystem_path: path,
            };
        }

        if directory_index_not_found && front_controller.is_none() {
            return HtaccessRouteEvaluationOutcome::NotFound;
        }

        if let HtaccessRouteBackend::Fastcgi { document_root, .. } = &request.backend {
            if serve_static.is_none() && fcgi_script_uri.is_none() {
                if let Some(fc) = front_controller.as_ref() {
                    match apply_front_controller(
                        self.probes.as_ref(),
                        document_root,
                        &request.request_path,
                        fc,
                    ) {
                        FrontControllerOutcome::Continue => {}
                        FrontControllerOutcome::RewriteToScript { script_uri } => {
                            fcgi_script_uri = Some(script_uri);
                        }
                        FrontControllerOutcome::InternalLoop => {
                            return HtaccessRouteEvaluationOutcome::Continue {
                                request_path,
                                uri_path,
                                fcgi_script_uri,
                                overlay_http_error: Some(500),
                            };
                        }
                        FrontControllerOutcome::TargetScriptMissing => {
                            return HtaccessRouteEvaluationOutcome::Continue {
                                request_path,
                                uri_path,
                                fcgi_script_uri,
                                overlay_http_error: Some(502),
                            };
                        }
                    }
                }
            }
        }

        HtaccessRouteEvaluationOutcome::Continue {
            request_path,
            uri_path,
            fcgi_script_uri,
            overlay_http_error: None,
        }
    }

    fn htaccess_overlay_merge_total(&self) -> u64 {
        OVERLAY_MERGE_TOTAL.load(Ordering::Relaxed)
    }

    fn htaccess_overlay_redirect_total(&self) -> u64 {
        OVERLAY_REDIRECT_TOTAL.load(Ordering::Relaxed)
    }

    fn htaccess_directory_index_hits_total(&self) -> u64 {
        DIRECTORY_INDEX_HITS.load(Ordering::Relaxed)
    }

    fn htaccess_directory_index_misses_total(&self) -> u64 {
        DIRECTORY_INDEX_MISSES.load(Ordering::Relaxed)
    }

    fn htaccess_front_controller_rewrites_total(&self) -> u64 {
        FRONT_CONTROLLER_REWRITES.load(Ordering::Relaxed)
    }

    fn htaccess_front_controller_bypass_file_total(&self) -> u64 {
        FRONT_CONTROLLER_BYPASS_FILE.load(Ordering::Relaxed)
    }

    fn htaccess_front_controller_bypass_directory_total(&self) -> u64 {
        FRONT_CONTROLLER_BYPASS_DIRECTORY.load(Ordering::Relaxed)
    }

    fn htaccess_internal_rewrite_loop_total(&self) -> u64 {
        INTERNAL_REWRITE_LOOP.load(Ordering::Relaxed)
    }
}

pub fn htaccess_overlay_merge_total() -> u64 {
    OVERLAY_MERGE_TOTAL.load(Ordering::Relaxed)
}

pub fn htaccess_overlay_redirect_total() -> u64 {
    OVERLAY_REDIRECT_TOTAL.load(Ordering::Relaxed)
}

pub fn htaccess_directory_index_hits_total() -> u64 {
    DIRECTORY_INDEX_HITS.load(Ordering::Relaxed)
}

pub fn htaccess_directory_index_misses_total() -> u64 {
    DIRECTORY_INDEX_MISSES.load(Ordering::Relaxed)
}

pub fn htaccess_front_controller_rewrites_total() -> u64 {
    FRONT_CONTROLLER_REWRITES.load(Ordering::Relaxed)
}

pub fn htaccess_front_controller_bypass_file_total() -> u64 {
    FRONT_CONTROLLER_BYPASS_FILE.load(Ordering::Relaxed)
}

pub fn htaccess_front_controller_bypass_directory_total() -> u64 {
    FRONT_CONTROLLER_BYPASS_DIRECTORY.load(Ordering::Relaxed)
}

pub fn htaccess_internal_rewrite_loop_total() -> u64 {
    INTERNAL_REWRITE_LOOP.load(Ordering::Relaxed)
}

pub fn append_htaccess_prometheus(out: &mut String) {
    out.push_str("# TYPE exyonq_htaccess_overlay_merge_total counter\n");
    out.push_str(&format!(
        "exyonq_htaccess_overlay_merge_total {}\n",
        htaccess_overlay_merge_total()
    ));
    out.push_str("# TYPE exyonq_htaccess_overlay_redirect_total counter\n");
    out.push_str(&format!(
        "exyonq_htaccess_overlay_redirect_total {}\n",
        htaccess_overlay_redirect_total()
    ));
    out.push_str("# TYPE exyonq_htaccess_directory_index_hits_total counter\n");
    out.push_str(&format!(
        "exyonq_htaccess_directory_index_hits_total {}\n",
        htaccess_directory_index_hits_total()
    ));
    out.push_str("# TYPE exyonq_htaccess_directory_index_misses_total counter\n");
    out.push_str(&format!(
        "exyonq_htaccess_directory_index_misses_total {}\n",
        htaccess_directory_index_misses_total()
    ));
}

enum FcgiDirIndex {
    Script(String),
    StaticFile {
        uri: String,
        filesystem_path: String,
    },
    Miss,
}

fn resolve_fcgi_directory_index(
    probes: &dyn HtaccessRuntimeProbes,
    document_root: &str,
    dir_uri: &str,
    candidates: &[String],
) -> FcgiDirIndex {
    for name in candidates.iter().take(MAX_DIRECTORY_INDEX_CANDIDATES) {
        if !validate_directory_index_candidate(name) {
            continue;
        }
        let Some(child_uri) = join_directory_index_uri(dir_uri, name) else {
            continue;
        };
        if name.ends_with(".php") {
            if probes.probe_fcgi_directory_index_script(document_root, dir_uri, &child_uri) {
                return FcgiDirIndex::Script(child_uri);
            }
            continue;
        }
        if let Some(path) = probes.probe_fcgi_directory_index_static_file(document_root, &child_uri)
        {
            return FcgiDirIndex::StaticFile {
                uri: child_uri,
                filesystem_path: path,
            };
        }
    }
    FcgiDirIndex::Miss
}

enum FrontControllerOutcome {
    Continue,
    RewriteToScript { script_uri: String },
    InternalLoop,
    TargetScriptMissing,
}

fn apply_front_controller(
    probes: &dyn HtaccessRuntimeProbes,
    document_root: &str,
    uri_path: &str,
    rule: &CompiledFrontController,
) -> FrontControllerOutcome {
    let target = rule.target_uri.as_ref();
    if uri_path == target {
        return match probes.probe_requested_resource(document_root, uri_path) {
            Ok(HtaccessResourceKind::File) => FrontControllerOutcome::Continue,
            Ok(HtaccessResourceKind::Directory) => {
                INTERNAL_REWRITE_LOOP.fetch_add(1, Ordering::Relaxed);
                FrontControllerOutcome::InternalLoop
            }
            Ok(HtaccessResourceKind::Missing) => FrontControllerOutcome::TargetScriptMissing,
            Err(()) => FrontControllerOutcome::TargetScriptMissing,
        };
    }

    match probes.probe_requested_resource(document_root, uri_path) {
        Ok(HtaccessResourceKind::File) => {
            FRONT_CONTROLLER_BYPASS_FILE.fetch_add(1, Ordering::Relaxed);
            FrontControllerOutcome::Continue
        }
        Ok(HtaccessResourceKind::Directory) => {
            FRONT_CONTROLLER_BYPASS_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            FrontControllerOutcome::Continue
        }
        Ok(HtaccessResourceKind::Missing) => {
            if !probes.fcgi_script_resolvable(document_root, target) {
                return FrontControllerOutcome::TargetScriptMissing;
            }
            FRONT_CONTROLLER_REWRITES.fetch_add(1, Ordering::Relaxed);
            FrontControllerOutcome::RewriteToScript {
                script_uri: target.to_string(),
            }
        }
        Err(()) => FrontControllerOutcome::TargetScriptMissing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_module_api::htaccess_runtime::HtaccessRuntimeProbes;
    use exyonq_module_api::{CompiledRedirectRule, NormalizedDirectory, OverlayEntry};
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MapSource(Mutex<HashMap<String, Arc<VhostOverlay>>>);

    impl HtaccessOverlaySource for MapSource {
        fn overlay_for_site(&self, site_id: &str) -> Option<Arc<VhostOverlay>> {
            self.0.lock().ok()?.get(site_id).cloned()
        }
    }

    struct StubProbes;

    impl HtaccessRuntimeProbes for StubProbes {
        fn probe_static_directory_index(
            &self,
            _root_slot: u32,
            _dir_uri: &str,
            _candidates: &[String],
        ) -> Option<String> {
            None
        }

        fn probe_fcgi_directory_index_script(
            &self,
            _document_root: &str,
            _dir_uri: &str,
            _candidate_uri: &str,
        ) -> bool {
            false
        }

        fn probe_fcgi_directory_index_static_file(
            &self,
            _document_root: &str,
            _child_uri: &str,
        ) -> Option<String> {
            None
        }

        fn probe_requested_resource(
            &self,
            _document_root: &str,
            _uri_path: &str,
        ) -> Result<HtaccessResourceKind, ()> {
            Ok(HtaccessResourceKind::Missing)
        }

        fn fcgi_script_resolvable(&self, _document_root: &str, _uri_path: &str) -> bool {
            false
        }
    }

    #[test]
    fn redirect_from_overlay_rules() {
        let overlay = Arc::new(VhostOverlay {
            generation: 1,
            site_id: "site".into(),
            document_root: "/var/www".into(),
            entries: Arc::from([OverlayEntry {
                directory: NormalizedDirectory("/".into()),
                directory_index: None,
                redirect_rules: Arc::from([CompiledRedirectRule {
                    from_path: "/old".into(),
                    status: 301,
                    location: "/new".into(),
                }]),
                rewrite_redirects: Arc::from([]),
                indexes_disabled: false,
                front_controller: None,
            }]),
        });
        let mut map = HashMap::new();
        map.insert("site".into(), overlay);
        let runtime =
            HtaccessRuntime::new(Arc::new(MapSource(Mutex::new(map))), Arc::new(StubProbes));
        let out = runtime.evaluate_route(&HtaccessRouteEvaluationRequest {
            site_id: "site".into(),
            request_path: "/old".into(),
            uri_path: "/old".into(),
            backend: HtaccessRouteBackend::Passthrough,
        });
        assert!(matches!(
            out,
            HtaccessRouteEvaluationOutcome::Redirect { status: 301, .. }
        ));
    }

    struct FsProbes;

    impl HtaccessRuntimeProbes for FsProbes {
        fn probe_static_directory_index(
            &self,
            _root_slot: u32,
            _dir_uri: &str,
            _candidates: &[String],
        ) -> Option<String> {
            None
        }

        fn probe_fcgi_directory_index_script(
            &self,
            document_root: &str,
            _dir_uri: &str,
            candidate_uri: &str,
        ) -> bool {
            let root = std::path::Path::new(document_root);
            let rel = candidate_uri.trim_start_matches('/');
            root.join(rel).is_file()
        }

        fn probe_fcgi_directory_index_static_file(
            &self,
            document_root: &str,
            child_uri: &str,
        ) -> Option<String> {
            let root = std::path::Path::new(document_root);
            let rel = child_uri.trim_start_matches('/');
            let path = root.join(rel);
            if path.is_file() {
                Some(path.to_string_lossy().into_owned())
            } else {
                None
            }
        }

        fn probe_requested_resource(
            &self,
            _document_root: &str,
            _uri_path: &str,
        ) -> Result<HtaccessResourceKind, ()> {
            Ok(HtaccessResourceKind::Missing)
        }

        fn fcgi_script_resolvable(&self, _document_root: &str, _uri_path: &str) -> bool {
            false
        }
    }

    #[test]
    fn fcgi_directory_index_prefers_php_script() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("app")).expect("mkdir");
        std::fs::write(dir.path().join("app/index.php"), b"<?php").expect("write");
        let outcome = resolve_fcgi_directory_index(
            &FsProbes,
            &dir.path().to_string_lossy(),
            "/app/",
            &["index.php".into(), "index.html".into()],
        );
        assert!(matches!(outcome, FcgiDirIndex::Script(uri) if uri == "/app/index.php"));
    }

    #[test]
    fn fcgi_directory_index_skips_missing_php_then_uses_html() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("app")).expect("mkdir");
        std::fs::write(dir.path().join("app/index.html"), b"html").expect("write");
        let outcome = resolve_fcgi_directory_index(
            &FsProbes,
            &dir.path().to_string_lossy(),
            "/app/",
            &["index.php".into(), "index.html".into()],
        );
        assert!(matches!(
            outcome,
            FcgiDirIndex::StaticFile { uri, .. } if uri == "/app/index.html"
        ));
    }
}
