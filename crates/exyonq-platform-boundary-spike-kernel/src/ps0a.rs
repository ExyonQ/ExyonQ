//! PS0A minimal planner tokens (superseded for canonical measurement by PS0B full-path).

/// Immutable view carried across the representative boundary (borrowed, no Arc).
#[derive(Clone, Copy, Debug)]
pub struct SpikeSnapshotView {
    pub generation: u64,
    pub modules_enabled: bool,
    pub site_static_slot: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WirePlanKind {
    Static,
    Proxy,
    HyperFallback,
}

/// Planner output tokens — stream/buffer ownership unchanged (head/rest stay borrowed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WirePlanTokens {
    pub kind: WirePlanKind,
    pub head_len: usize,
    pub rest_len: usize,
    pub generation: u64,
}

#[inline]
fn spike_might_use_proxy_wire(head: &[u8]) -> bool {
    if !(head.starts_with(b"GET /api/") || head.starts_with(b"GET /api ")) {
        return false;
    }
    if head
        .windows(b"Upgrade: websocket".len())
        .any(|w| w.eq_ignore_ascii_case(b"upgrade: websocket"))
    {
        return false;
    }
    true
}

#[inline]
fn spike_might_use_static_wire(head: &[u8]) -> bool {
    head.starts_with(b"GET /health ")
        || head.starts_with(b"HEAD /health ")
        || head.starts_with(b"GET /site/1k.bin ")
        || head.starts_with(b"HEAD /site/1k.bin ")
        || head.starts_with(b"GET /site/64k.bin ")
        || head.starts_with(b"HEAD /site/64k.bin ")
        || head.starts_with(b"GET /site/1m.bin ")
        || head.starts_with(b"HEAD /site/1m.bin ")
        || head.starts_with(b"GET /site/routes/route")
        || head.starts_with(b"HEAD /site/routes/route")
        || head.starts_with(b"GET /metrics ")
        || head.starts_with(b"GET /metrics\r")
}

/// Semantic planner — PS0 §22.1 callee (PS0A scope).
#[inline(never)]
pub fn plan_wire_after_headers(
    head: &[u8],
    rest: &[u8],
    state: &SpikeSnapshotView,
) -> Option<WirePlanTokens> {
    if head.is_empty() {
        return None;
    }

    let kind = if !state.modules_enabled {
        if spike_might_use_proxy_wire(head) {
            WirePlanKind::Proxy
        } else if state.site_static_slot.is_some() && spike_might_use_static_wire(head) {
            WirePlanKind::Static
        } else {
            WirePlanKind::HyperFallback
        }
    } else {
        WirePlanKind::HyperFallback
    };

    Some(WirePlanTokens {
        kind,
        head_len: head.len(),
        rest_len: rest.len(),
        generation: state.generation,
    })
}

pub type PlanFn = fn(&[u8], &[u8], &SpikeSnapshotView) -> Option<WirePlanTokens>;

#[inline(always)]
pub fn plan_fn_ptr() -> PlanFn {
    plan_wire_after_headers
}
