//! PS0A/PS0B experimental kernel planner — NOT production architecture.

mod full_path;
mod ps0a;

pub use full_path::{
    plan_wire_after_headers_arc, plan_wire_after_headers_borrowed, plan_wire_after_headers_full,
    FullPlanFn, OpaquePlanRegistry, SpikeGenerationSnap, SpikeSnapshotArc, SpikeSnapshotBorrowed,
    SpikeStream, SpikeWirePlan, SpikeWirePlanKind, KERNEL_OPAQUE_PLAN,
};
pub use ps0a::{
    plan_fn_ptr, plan_wire_after_headers, PlanFn, SpikeSnapshotView, WirePlanKind, WirePlanTokens,
};
