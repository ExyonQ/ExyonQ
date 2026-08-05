//! PS0B full-path kernel planner — `WirePlan<S>` + `Bytes` + owned stream.

use std::sync::{Arc, OnceLock};

use bytes::Bytes;

/// Owned non-`Copy` stream token with optional drop tracking (tests / protector).
pub struct SpikeStream {
    pub id: u64,
    drop_tracker: Option<Arc<std::sync::atomic::AtomicU32>>,
}

impl SpikeStream {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            drop_tracker: None,
        }
    }

    pub fn with_drop_tracker(id: u64, tracker: Arc<std::sync::atomic::AtomicU32>) -> Self {
        Self {
            id,
            drop_tracker: Some(tracker),
        }
    }
}

impl Drop for SpikeStream {
    fn drop(&mut self) {
        if let Some(t) = &self.drop_tracker {
            t.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpikeWirePlanKind {
    Static,
    Proxy,
    Hyper,
}

/// Mirrors production `WirePlan<S>` shape (spike-local names).
pub enum SpikeWirePlan<S> {
    Static(S, Bytes, Bytes),
    Proxy(S, Bytes, Bytes),
    Hyper(S, Bytes, Bytes),
}

impl<S> SpikeWirePlan<S> {
    pub fn kind(&self) -> SpikeWirePlanKind {
        match self {
            SpikeWirePlan::Static(..) => SpikeWirePlanKind::Static,
            SpikeWirePlan::Proxy(..) => SpikeWirePlanKind::Proxy,
            SpikeWirePlan::Hyper(..) => SpikeWirePlanKind::Hyper,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpikeSnapshotBorrowed {
    pub generation: u64,
    pub modules_enabled: bool,
    pub site_static_slot: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct SpikeGenerationSnap {
    pub generation: u64,
    pub modules_enabled: bool,
    pub site_static_slot: Option<u32>,
}

#[derive(Clone)]
pub struct SpikeSnapshotArc(pub Arc<SpikeGenerationSnap>);

impl SpikeSnapshotBorrowed {
    pub fn from_arc(snap: &SpikeSnapshotArc) -> Self {
        Self {
            generation: snap.0.generation,
            modules_enabled: snap.0.modules_enabled,
            site_static_slot: snap.0.site_static_slot,
        }
    }
}

fn spike_might_use_proxy_wire(head: &Bytes) -> bool {
    if !(head.starts_with(b"GET /api/") || head.starts_with(b"GET /api ")) {
        return false;
    }
    !head
        .as_ref()
        .windows(b"Upgrade: websocket".len())
        .any(|w| w.eq_ignore_ascii_case(b"upgrade: websocket"))
}

fn spike_might_use_static_wire(head: &Bytes) -> bool {
    let h = head.as_ref();
    h.starts_with(b"GET /health ")
        || h.starts_with(b"HEAD /health ")
        || h.starts_with(b"GET /site/1k.bin ")
        || h.starts_with(b"HEAD /site/1k.bin ")
        || h.starts_with(b"GET /site/64k.bin ")
        || h.starts_with(b"HEAD /site/64k.bin ")
        || h.starts_with(b"GET /site/1m.bin ")
        || h.starts_with(b"HEAD /site/1m.bin ")
        || h.starts_with(b"GET /site/routes/route")
        || h.starts_with(b"HEAD /site/routes/route")
        || h.starts_with(b"GET /metrics ")
        || h.starts_with(b"GET /metrics\r")
}

fn classify_wire_plan<S>(
    stream: S,
    head: Bytes,
    rest: Bytes,
    snap: SpikeSnapshotBorrowed,
) -> Option<SpikeWirePlan<S>> {
    if head.is_empty() {
        return None;
    }

    if !snap.modules_enabled {
        if spike_might_use_proxy_wire(&head) {
            return Some(SpikeWirePlan::Proxy(stream, head, rest));
        }
        if snap.site_static_slot.is_some() && spike_might_use_static_wire(&head) {
            return Some(SpikeWirePlan::Static(stream, head, rest));
        }
    }

    Some(SpikeWirePlan::Hyper(stream, head, rest))
}

/// Cross-crate callee — borrowed snapshot (no `Arc` clone in planner).
#[inline(never)]
pub fn plan_wire_after_headers_borrowed(
    stream: SpikeStream,
    head: Bytes,
    rest: Bytes,
    snap: &SpikeSnapshotBorrowed,
) -> Option<SpikeWirePlan<SpikeStream>> {
    plan_wire_after_headers_full(stream, head, rest, *snap)
}

/// Cross-crate callee — `Arc` snapshot handle (planner borrows inner; no clone).
#[inline(never)]
pub fn plan_wire_after_headers_arc(
    stream: SpikeStream,
    head: Bytes,
    rest: Bytes,
    snap: &SpikeSnapshotArc,
) -> Option<SpikeWirePlan<SpikeStream>> {
    let borrowed = SpikeSnapshotBorrowed::from_arc(snap);
    classify_wire_plan(stream, head, rest, borrowed)
}

#[inline(never)]
pub fn plan_wire_after_headers_full(
    stream: SpikeStream,
    head: Bytes,
    rest: Bytes,
    snap: SpikeSnapshotBorrowed,
) -> Option<SpikeWirePlan<SpikeStream>> {
    classify_wire_plan(stream, head, rest, snap)
}

pub type FullPlanFn =
    fn(SpikeStream, Bytes, Bytes, &SpikeSnapshotBorrowed) -> Option<SpikeWirePlan<SpikeStream>>;

/// Opaque `OnceLock` registration — production-like fn pointer table.
pub struct OpaquePlanRegistry {
    plan: OnceLock<FullPlanFn>,
}

impl Default for OpaquePlanRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl OpaquePlanRegistry {
    pub const fn new() -> Self {
        Self {
            plan: OnceLock::new(),
        }
    }

    pub fn ensure_registered(&self) -> FullPlanFn {
        *self
            .plan
            .get_or_init(|| plan_wire_after_headers_borrowed as FullPlanFn)
    }

    #[inline(never)]
    pub fn call_borrowed(
        &self,
        stream: SpikeStream,
        head: Bytes,
        rest: Bytes,
        snap: &SpikeSnapshotBorrowed,
    ) -> Option<SpikeWirePlan<SpikeStream>> {
        let f = self.ensure_registered();
        f(stream, head, rest, snap)
    }
}

pub static KERNEL_OPAQUE_PLAN: OpaquePlanRegistry = OpaquePlanRegistry::new();
