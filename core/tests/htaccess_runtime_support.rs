//! Test helper — per-test htaccess runtime via RAII TLS override.

use exyonq_core::{CoreHtaccessProbes, HtaccessRuntimeTestGuard};
use exyonq_mod_htaccess::{HtaccessRuntime, OverlayPublisher};
use exyonq_module_api::fcgi_script_resolver::FastcgiScriptResolverTestGuard;
use std::sync::Arc;

/// RAII bundle — guards drop in field order when the struct is held.
#[allow(dead_code)]
pub struct HtaccessRuntimeInstall {
    pub htaccess: HtaccessRuntimeTestGuard,
    pub script_resolver: FastcgiScriptResolverTestGuard,
}

pub fn install_htaccess_runtime_publisher(
    publisher: Arc<OverlayPublisher>,
) -> HtaccessRuntimeInstall {
    let script_resolver =
        FastcgiScriptResolverTestGuard::install(exyonq_mod_fastcgi::FastcgiScriptResolver::arc());
    let source: Arc<dyn exyonq_module_api::htaccess_runtime::HtaccessOverlaySource> = publisher;
    let runtime = Arc::new(HtaccessRuntime::new(source, Arc::new(CoreHtaccessProbes)));
    let htaccess = HtaccessRuntimeTestGuard::install(runtime);
    HtaccessRuntimeInstall {
        htaccess,
        script_resolver,
    }
}
