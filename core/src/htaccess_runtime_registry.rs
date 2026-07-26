//! Htaccess overlay runtime service slot (KD4.2 — module-owned policy).

use crate::contract_service_registry::{
    ContractServiceSlot, ContractServiceTestGuard, TestServiceOverride,
};
use exyonq_module_api::htaccess_runtime::{
    HtaccessRegisterError, HtaccessRouteBackend, HtaccessRouteEvaluationOutcome,
    HtaccessRouteEvaluationRequest, HtaccessRuntimeService,
};
use std::cell::RefCell;
use std::sync::Arc;

static HTACCESS_RUNTIME_SLOT: ContractServiceSlot<dyn HtaccessRuntimeService> =
    ContractServiceSlot::new();

thread_local! {
    static HTACCESS_RUNTIME_TLS: RefCell<TestServiceOverride<dyn HtaccessRuntimeService>> =
        const { RefCell::new(TestServiceOverride::Inherit) };
}

pub fn register_htaccess_runtime_service(
    service: Arc<dyn HtaccessRuntimeService>,
) -> Result<(), HtaccessRegisterError> {
    HTACCESS_RUNTIME_SLOT.register_htaccess(service)
}

fn resolve_service() -> Option<Arc<dyn HtaccessRuntimeService>> {
    HTACCESS_RUNTIME_SLOT.resolve(&HTACCESS_RUNTIME_TLS.with(|c| c.borrow().clone()))
}

pub struct HtaccessRuntimeTestGuard {
    _inner: ContractServiceTestGuard<dyn HtaccessRuntimeService>,
}

impl HtaccessRuntimeTestGuard {
    pub fn install(service: Arc<dyn HtaccessRuntimeService>) -> Self {
        Self {
            _inner: ContractServiceTestGuard::install(&HTACCESS_RUNTIME_TLS, service),
        }
    }
}

fn evaluate_htaccess_route(
    request: HtaccessRouteEvaluationRequest,
) -> Option<HtaccessRouteEvaluationOutcome> {
    let service = resolve_service()?;
    Some(service.evaluate_route(&request))
}

/// Mechanical result for handler after overlay runtime evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtaccessDispatchAdjustment {
    pub redirect_status: Option<u16>,
    pub redirect_location: Option<String>,
    pub serve_static_file: Option<String>,
    pub overlay_http_error: Option<u16>,
    pub terminal_not_found: bool,
    pub request_path: String,
    pub uri_path: String,
    pub fcgi_script_uri: Option<String>,
}

pub fn htaccess_adjust_route(
    snapshot: &crate::RuntimePlan,
    route_idx: usize,
    site_id: &str,
    path: &str,
    uri_path: &str,
) -> Option<HtaccessDispatchAdjustment> {
    let outcome = evaluate_htaccess_route(HtaccessRouteEvaluationRequest {
        site_id: site_id.to_string(),
        request_path: path.to_string(),
        uri_path: uri_path.to_string(),
        backend: build_htaccess_route_backend(snapshot, route_idx),
    })?;
    Some(match outcome {
        HtaccessRouteEvaluationOutcome::NoOverlay => HtaccessDispatchAdjustment {
            redirect_status: None,
            redirect_location: None,
            serve_static_file: None,
            overlay_http_error: None,
            terminal_not_found: false,
            request_path: path.to_string(),
            uri_path: uri_path.to_string(),
            fcgi_script_uri: None,
        },
        HtaccessRouteEvaluationOutcome::Redirect { status, location } => {
            HtaccessDispatchAdjustment {
                redirect_status: Some(status),
                redirect_location: Some(location),
                serve_static_file: None,
                overlay_http_error: None,
                terminal_not_found: false,
                request_path: path.to_string(),
                uri_path: uri_path.to_string(),
                fcgi_script_uri: None,
            }
        }
        HtaccessRouteEvaluationOutcome::ServeStaticFile { filesystem_path } => {
            HtaccessDispatchAdjustment {
                redirect_status: None,
                redirect_location: None,
                serve_static_file: Some(filesystem_path),
                overlay_http_error: None,
                terminal_not_found: false,
                request_path: path.to_string(),
                uri_path: uri_path.to_string(),
                fcgi_script_uri: None,
            }
        }
        HtaccessRouteEvaluationOutcome::NotFound => HtaccessDispatchAdjustment {
            redirect_status: None,
            redirect_location: None,
            serve_static_file: None,
            overlay_http_error: None,
            terminal_not_found: true,
            request_path: path.to_string(),
            uri_path: uri_path.to_string(),
            fcgi_script_uri: None,
        },
        HtaccessRouteEvaluationOutcome::Continue {
            request_path,
            uri_path,
            fcgi_script_uri,
            overlay_http_error,
        } => HtaccessDispatchAdjustment {
            redirect_status: None,
            redirect_location: None,
            serve_static_file: None,
            overlay_http_error,
            terminal_not_found: false,
            request_path,
            uri_path,
            fcgi_script_uri,
        },
    })
}

pub fn build_htaccess_route_backend(
    snapshot: &crate::RuntimePlan,
    route_idx: usize,
) -> HtaccessRouteBackend {
    use crate::Backend;
    match snapshot.resolve_backend(route_idx) {
        Some(Backend::Static { root_slot }) => HtaccessRouteBackend::Static {
            root_slot: *root_slot,
        },
        Some(Backend::Fastcgi { pool_id }) => HtaccessRouteBackend::Fastcgi {
            pool_id: *pool_id,
            document_root: snapshot
                .fcgi_pool_document_root(*pool_id)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        },
        _ => HtaccessRouteBackend::Passthrough,
    }
}

#[doc(hidden)]
pub fn clear_htaccess_runtime_for_tests() {
    HTACCESS_RUNTIME_SLOT.clear_for_tests();
}
