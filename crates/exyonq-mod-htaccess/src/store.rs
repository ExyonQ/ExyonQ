//! Atomic overlay publication store (composition / core read path).

use arc_swap::ArcSwap;
use exyonq_module_api::{
    validate_publish, OverlayPublishError, RuntimePatchVhostOverlay, VhostOverlay,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub struct OverlayPublisher {
    plan_generation: AtomicU64,
    global_generation: AtomicU64,
    sites: ArcSwap<HashMap<String, Arc<VhostOverlay>>>,
    reload_success: AtomicU64,
    reload_failure: AtomicU64,
    files_compiled: AtomicU64,
    unsupported_total: AtomicU64,
}

impl OverlayPublisher {
    pub fn new(plan_generation: u64) -> Self {
        Self {
            plan_generation: AtomicU64::new(plan_generation),
            global_generation: AtomicU64::new(0),
            sites: ArcSwap::from_pointee(HashMap::new()),
            reload_success: AtomicU64::new(0),
            reload_failure: AtomicU64::new(0),
            files_compiled: AtomicU64::new(0),
            unsupported_total: AtomicU64::new(0),
        }
    }

    pub fn set_plan_generation(&self, generation: u64) {
        self.plan_generation.store(generation, Ordering::SeqCst);
    }

    pub fn plan_generation(&self) -> u64 {
        self.plan_generation.load(Ordering::SeqCst)
    }

    pub fn overlay_generation(&self) -> u64 {
        self.global_generation.load(Ordering::SeqCst)
    }

    pub fn get(&self, site_id: &str) -> Option<Arc<VhostOverlay>> {
        self.sites.load().get(site_id).cloned()
    }

    pub fn publish(&self, patch: RuntimePatchVhostOverlay) -> Result<u64, OverlayPublishError> {
        validate_publish(&patch, self.plan_generation())?;
        let mut next = (**self.sites.load()).clone();
        next.insert(patch.site_id.clone(), Arc::new(patch.overlay));
        self.sites.store(Arc::new(next));
        let gen = self.global_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.reload_success.fetch_add(1, Ordering::Relaxed);
        Ok(gen)
    }

    pub fn record_compile_failure(&self) {
        self.reload_failure.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_files_compiled(&self, count: u64) {
        self.files_compiled.fetch_add(count, Ordering::Relaxed);
    }

    pub fn record_unsupported(&self, count: u64) {
        self.unsupported_total.fetch_add(count, Ordering::Relaxed);
    }

    pub fn metrics_snapshot(&self) -> OverlayMetrics {
        OverlayMetrics {
            overlay_generation: self.overlay_generation(),
            reload_success_total: self.reload_success.load(Ordering::Relaxed),
            reload_failure_total: self.reload_failure.load(Ordering::Relaxed),
            files_compiled: self.files_compiled.load(Ordering::Relaxed),
            directives_unsupported_total: self.unsupported_total.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayMetrics {
    pub overlay_generation: u64,
    pub reload_success_total: u64,
    pub reload_failure_total: u64,
    pub files_compiled: u64,
    pub directives_unsupported_total: u64,
}
