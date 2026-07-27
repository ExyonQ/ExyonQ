//! PS0B full-path platform caller + monolith M2 planner (harness crate only).

use bytes::Bytes;
use exyonq_platform_boundary_spike_kernel::{
    plan_wire_after_headers_borrowed, SpikeSnapshotArc, SpikeSnapshotBorrowed, SpikeStream,
    SpikeWirePlan, SpikeWirePlanKind, KERNEL_OPAQUE_PLAN,
};

pub mod fixtures {
    use super::*;
    use bytes::Bytes;
    use exyonq_platform_boundary_spike_kernel::SpikeGenerationSnap;
    use std::sync::Arc;

    pub const HEAD_P1_STATIC: &[u8] = b"GET /site/1k.bin HTTP/1.1\r\nHost: bench\r\n\r\n";
    pub const HEAD_P1_PROXY: &[u8] = b"GET /api/echo HTTP/1.1\r\nHost: bench\r\n\r\n";
    pub const HEAD_HYPER_FALLBACK: &[u8] = b"POST /submit HTTP/1.1\r\nHost: bench\r\n\r\n";
    pub const REST_EMPTY: &[u8] = &[];

    pub const BORROWED_SNAP: SpikeSnapshotBorrowed = SpikeSnapshotBorrowed {
        generation: 42,
        modules_enabled: false,
        site_static_slot: Some(0),
    };

    pub fn borrowed_snap() -> SpikeSnapshotBorrowed {
        BORROWED_SNAP
    }

    pub fn arc_snap() -> SpikeSnapshotArc {
        SpikeSnapshotArc(Arc::new(SpikeGenerationSnap {
            generation: 42,
            modules_enabled: false,
            site_static_slot: Some(0),
        }))
    }

    pub fn head_static() -> Bytes {
        Bytes::from_static(HEAD_P1_STATIC)
    }

    pub fn head_proxy() -> Bytes {
        Bytes::from_static(HEAD_P1_PROXY)
    }

    pub fn head_hyper() -> Bytes {
        Bytes::from_static(HEAD_HYPER_FALLBACK)
    }

    pub fn rest_empty() -> Bytes {
        Bytes::from_static(REST_EMPTY)
    }

    pub fn stream(id: u64) -> SpikeStream {
        SpikeStream::new(id)
    }
}

pub struct FixtureSet {
    pub head: Bytes,
    pub rest: Bytes,
    pub snap: SpikeSnapshotBorrowed,
    pub expected: SpikeWirePlanKind,
}

#[allow(dead_code)]
pub fn fixture_shapes() -> [FixtureSet; 3] {
    [
        FixtureSet {
            head: fixtures::head_static(),
            rest: fixtures::rest_empty(),
            snap: fixtures::borrowed_snap(),
            expected: SpikeWirePlanKind::Static,
        },
        FixtureSet {
            head: fixtures::head_proxy(),
            rest: fixtures::rest_empty(),
            snap: fixtures::borrowed_snap(),
            expected: SpikeWirePlanKind::Proxy,
        },
        FixtureSet {
            head: fixtures::head_hyper(),
            rest: fixtures::rest_empty(),
            snap: fixtures::borrowed_snap(),
            expected: SpikeWirePlanKind::Hyper,
        },
    ]
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

/// M2 monolith planner — same crate as caller; distinct symbol from kernel export.
#[inline(never)]
pub fn plan_wire_monolith_full(
    stream: SpikeStream,
    head: Bytes,
    rest: Bytes,
    snap: SpikeSnapshotBorrowed,
) -> Option<SpikeWirePlan<SpikeStream>> {
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

/// Platform-side read → M2 monolith planner.
#[inline(never)]
pub fn read_wire_plan_m2(
    stream: SpikeStream,
    head: Bytes,
    rest: Bytes,
    snap: &SpikeSnapshotBorrowed,
) -> Option<SpikeWirePlan<SpikeStream>> {
    plan_wire_monolith_full(stream, head, rest, *snap)
}

/// Platform-side read → X2 cross-crate static planner.
#[inline(never)]
pub fn read_wire_plan_x2(
    stream: SpikeStream,
    head: Bytes,
    rest: Bytes,
    snap: &SpikeSnapshotBorrowed,
) -> Option<SpikeWirePlan<SpikeStream>> {
    plan_wire_after_headers_borrowed(stream, head, rest, snap)
}

/// Platform-side read → F2 opaque `OnceLock` fn pointer.
#[inline(never)]
pub fn read_wire_plan_f2(
    stream: SpikeStream,
    head: Bytes,
    rest: Bytes,
    snap: &SpikeSnapshotBorrowed,
) -> Option<SpikeWirePlan<SpikeStream>> {
    KERNEL_OPAQUE_PLAN.call_borrowed(stream, head, rest, snap)
}

/// Representative static-arm token after plan selection (no I/O).
#[inline(never)]
pub fn static_response_token(head: &Bytes) -> u64 {
    head.len() as u64 ^ 0x5A71C
}

/// Consume plan + exercise handoff branch (drops stream once).
#[inline(never)]
pub fn consume_wire_plan(plan: SpikeWirePlan<SpikeStream>) -> u64 {
    match plan {
        SpikeWirePlan::Static(stream, head, rest) => {
            let tok = static_response_token(&head);
            tok ^ stream.id ^ rest.len() as u64
        }
        SpikeWirePlan::Proxy(stream, head, rest) => {
            stream.id ^ head.len() as u64 ^ rest.len() as u64 ^ 0x50A2
        }
        SpikeWirePlan::Hyper(stream, head, rest) => {
            stream.id ^ head.len() as u64 ^ rest.len() as u64 ^ 0x4859
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_platform_boundary_spike_kernel::plan_wire_after_headers_arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    fn assert_variants_match(f: &FixtureSet) {
        let snap = f.snap;
        let m =
            read_wire_plan_m2(fixtures::stream(1), f.head.clone(), f.rest.clone(), &snap).unwrap();
        let x =
            read_wire_plan_x2(fixtures::stream(2), f.head.clone(), f.rest.clone(), &snap).unwrap();
        let fp =
            read_wire_plan_f2(fixtures::stream(3), f.head.clone(), f.rest.clone(), &snap).unwrap();
        assert_eq!(m.kind(), f.expected);
        assert_eq!(x.kind(), f.expected);
        assert_eq!(fp.kind(), f.expected);
    }

    #[test]
    fn semantics_m2_x2_f2_match() {
        for f in fixture_shapes() {
            assert_variants_match(&f);
        }
    }

    #[test]
    fn stream_drop_exactly_once() {
        let drops = Arc::new(AtomicU32::new(0));
        let stream = SpikeStream::with_drop_tracker(7, Arc::clone(&drops));
        let plan = read_wire_plan_m2(
            stream,
            fixtures::head_static(),
            fixtures::rest_empty(),
            &fixtures::borrowed_snap(),
        )
        .unwrap();
        let _ = consume_wire_plan(plan);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn buffer_and_generation_preserved() {
        let head = fixtures::head_static();
        let rest = fixtures::rest_empty();
        let snap = fixtures::borrowed_snap();
        let plan =
            read_wire_plan_x2(fixtures::stream(9), head.clone(), rest.clone(), &snap).unwrap();
        match plan {
            SpikeWirePlan::Static(_, h, r) => {
                assert_eq!(h, head);
                assert_eq!(r, rest);
            }
            _ => panic!("expected static"),
        }
        assert_eq!(snap.generation, 42);
    }

    #[test]
    fn arc_snap_no_extra_clone_in_planner() {
        let arc = fixtures::arc_snap();
        let before = Arc::strong_count(&arc.0);
        let plan = plan_wire_after_headers_arc(
            fixtures::stream(4),
            fixtures::head_proxy(),
            fixtures::rest_empty(),
            &arc,
        )
        .unwrap();
        assert_eq!(plan.kind(), SpikeWirePlanKind::Proxy);
        assert_eq!(Arc::strong_count(&arc.0), before);
    }
}
