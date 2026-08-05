//! PS0A harness (legacy micro-bench); canonical PS0B uses `full_path` + separate bins.

pub use exyonq_platform_boundary_spike_kernel::{
    plan_wire_after_headers, PlanFn, SpikeSnapshotView, WirePlanKind, WirePlanTokens,
};

/// Representative header shapes (P1 static, proxy, hyper fallback).
pub const HEAD_P1_STATIC: &[u8] = b"GET /site/1k.bin HTTP/1.1\r\nHost: bench\r\n\r\n";
pub const HEAD_P1_PROXY: &[u8] = b"GET /api/echo HTTP/1.1\r\nHost: bench\r\n\r\n";
pub const HEAD_HYPER_FALLBACK: &[u8] = b"POST /submit HTTP/1.1\r\nHost: bench\r\n\r\n";
pub const REST_EMPTY: &[u8] = &[];

pub const BENCH_STATE: SpikeSnapshotView = SpikeSnapshotView {
    generation: 42,
    modules_enabled: false,
    site_static_slot: Some(0),
};

#[inline(never)]
pub fn read_wire_plan_monolith(
    head: &[u8],
    rest: &[u8],
    state: &SpikeSnapshotView,
) -> Option<WirePlanTokens> {
    plan_wire_monolith_inline(head, rest, state)
}

#[inline(never)]
fn plan_wire_monolith_inline(
    head: &[u8],
    rest: &[u8],
    state: &SpikeSnapshotView,
) -> Option<WirePlanTokens> {
    if head.is_empty() {
        return None;
    }
    let kind = if !state.modules_enabled {
        if spike_might_use_proxy_wire_local(head) {
            WirePlanKind::Proxy
        } else if state.site_static_slot.is_some() && spike_might_use_static_wire_local(head) {
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

#[inline(never)]
pub fn read_wire_plan_cross_static(
    head: &[u8],
    rest: &[u8],
    state: &SpikeSnapshotView,
) -> Option<WirePlanTokens> {
    plan_wire_after_headers(head, rest, state)
}

#[inline(never)]
pub fn read_wire_plan_cross_fnptr(
    head: &[u8],
    rest: &[u8],
    state: &SpikeSnapshotView,
    plan: PlanFn,
) -> Option<WirePlanTokens> {
    let plan = core::hint::black_box(plan);
    plan(head, rest, state)
}

#[inline]
fn spike_might_use_proxy_wire_local(head: &[u8]) -> bool {
    if !(head.starts_with(b"GET /api/") || head.starts_with(b"GET /api ")) {
        return false;
    }
    !head
        .windows(b"Upgrade: websocket".len())
        .any(|w| w.eq_ignore_ascii_case(b"upgrade: websocket"))
}

#[inline]
fn spike_might_use_static_wire_local(head: &[u8]) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_platform_boundary_spike_kernel::plan_fn_ptr;

    fn assert_triple_equal(head: &[u8]) {
        let m = read_wire_plan_monolith(head, REST_EMPTY, &BENCH_STATE).unwrap();
        let x = read_wire_plan_cross_static(head, REST_EMPTY, &BENCH_STATE).unwrap();
        let f = read_wire_plan_cross_fnptr(head, REST_EMPTY, &BENCH_STATE, plan_fn_ptr()).unwrap();
        assert_eq!(m.kind, x.kind);
        assert_eq!(m.kind, f.kind);
    }

    #[test]
    fn semantics_static_proxy_hyper_match() {
        assert_triple_equal(HEAD_P1_STATIC);
        assert_triple_equal(HEAD_P1_PROXY);
        assert_triple_equal(HEAD_HYPER_FALLBACK);
    }
}
