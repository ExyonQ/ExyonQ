//! Neutral cache consumer id (backends define their own constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheNamespace(pub u16);

impl CacheNamespace {
    pub const fn new(id: u16) -> Self {
        Self(id)
    }
}

/// Per-namespace metric hooks (no backend names in store logic).
#[derive(Clone, Copy)]
pub struct NamespaceMetrics {
    pub on_hit: fn(),
    pub on_miss: fn(),
    pub on_insert: fn(),
    pub on_rejection: fn(),
}

impl NamespaceMetrics {
    pub const NONE: Self = Self {
        on_hit: || {},
        on_miss: || {},
        on_insert: || {},
        on_rejection: || {},
    };
}

/// Result of optional hit-time validation (e.g. static identity revalidation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitValidation {
    Accept,
    Reject { entry_id: u64 },
}

pub type HitValidator = fn(&crate::store::CachedEntry) -> HitValidation;

#[derive(Clone, Copy)]
pub struct ServeHooks {
    pub namespace: CacheNamespace,
    pub metrics: NamespaceMetrics,
    pub validate_hit: Option<HitValidator>,
}

impl ServeHooks {
    pub const fn new(namespace: CacheNamespace, metrics: NamespaceMetrics) -> Self {
        Self {
            namespace,
            metrics,
            validate_hit: None,
        }
    }
}
