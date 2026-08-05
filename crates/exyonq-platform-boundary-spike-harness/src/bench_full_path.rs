//! PS0B statistical bench driver — separate binaries per variant (symmetric control).

use std::hint::black_box;
use std::time::Instant;

use bytes::Bytes;
use exyonq_platform_boundary_spike_kernel::{
    SpikeSnapshotBorrowed, SpikeStream, SpikeWirePlan, SpikeWirePlanKind,
};

use crate::full_path::{
    consume_wire_plan, fixtures, read_wire_plan_f2, read_wire_plan_m2, read_wire_plan_x2,
};

pub const FULL_PATH_ITERS: u32 = 2_000_000;
pub const FULL_PATH_WARMUP: u32 = 100_000;
pub const FULL_PATH_RUNS: usize = 5;

#[derive(Clone, Copy, Debug)]
pub enum BenchVariant {
    M2,
    X2,
    F2,
    N2,
}

#[derive(Clone, Copy, Debug)]
pub enum BenchMode {
    PlannerOnly,
    WirePlanConstruct,
    FullConsume,
}

#[derive(Clone, Copy, Debug)]
pub enum BenchShape {
    P1Static,
    P1Proxy,
    HyperFallback,
}

impl BenchShape {
    fn head_rest(self) -> (Bytes, Bytes) {
        match self {
            BenchShape::P1Static => (fixtures::head_static(), fixtures::rest_empty()),
            BenchShape::P1Proxy => (fixtures::head_proxy(), fixtures::rest_empty()),
            BenchShape::HyperFallback => (fixtures::head_hyper(), fixtures::rest_empty()),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BenchShape::P1Static => "P1_static",
            BenchShape::P1Proxy => "P1_proxy",
            BenchShape::HyperFallback => "hyper_fallback",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BenchStats {
    pub median_ns: f64,
    pub min_ns: f64,
    pub max_ns: f64,
    pub cv_percent: f64,
    pub samples: Vec<f64>,
}

impl BenchStats {
    pub fn from_samples(mut samples: Vec<f64>) -> Self {
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = samples.len();
        let median_ns = samples[n / 2];
        let min_ns = samples[0];
        let max_ns = samples[n - 1];
        let mean = samples.iter().sum::<f64>() / n as f64;
        let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
        let cv_percent = if mean > 0.0 {
            (var.sqrt() / mean) * 100.0
        } else {
            0.0
        };
        Self {
            median_ns,
            min_ns,
            max_ns,
            cv_percent,
            samples,
        }
    }

    pub fn print(&self, label: &str) {
        println!(
            "{label}: median={:.2} min={:.2} max={:.2} cv={:.1}% runs={}",
            self.median_ns,
            self.min_ns,
            self.max_ns,
            self.cv_percent,
            self.samples.len()
        );
    }
}

fn dispatch_plan(
    variant: BenchVariant,
    stream: SpikeStream,
    head: Bytes,
    rest: Bytes,
    snap: &SpikeSnapshotBorrowed,
) -> Option<SpikeWirePlan<SpikeStream>> {
    match variant {
        BenchVariant::M2 | BenchVariant::N2 => read_wire_plan_m2(stream, head, rest, snap),
        BenchVariant::X2 => read_wire_plan_x2(stream, head, rest, snap),
        BenchVariant::F2 => read_wire_plan_f2(stream, head, rest, snap),
    }
}

fn run_once_ns(
    variant: BenchVariant,
    mode: BenchMode,
    shape: BenchShape,
    iters: u32,
    warmup: u32,
) -> f64 {
    let (head, rest) = shape.head_rest();
    let snap = fixtures::borrowed_snap();

    for i in 0..warmup {
        let stream = SpikeStream::new(i as u64);
        match mode {
            BenchMode::PlannerOnly => {
                let k = dispatch_plan(variant, stream, head.clone(), rest.clone(), &snap)
                    .map(|p| p.kind());
                black_box(k);
            }
            BenchMode::WirePlanConstruct => {
                let p = dispatch_plan(variant, stream, head.clone(), rest.clone(), &snap);
                black_box(p.map(|x| x.kind()));
            }
            BenchMode::FullConsume => {
                if let Some(plan) =
                    dispatch_plan(variant, stream, head.clone(), rest.clone(), &snap)
                {
                    black_box(consume_wire_plan(plan));
                }
            }
        }
    }

    let start = Instant::now();
    for i in 0..iters {
        let stream = SpikeStream::new((i as u64) | 0x1_0000);
        match mode {
            BenchMode::PlannerOnly => {
                let k = dispatch_plan(variant, stream, head.clone(), rest.clone(), &snap)
                    .map(|p| p.kind());
                black_box(k);
            }
            BenchMode::WirePlanConstruct => {
                let p = dispatch_plan(variant, stream, head.clone(), rest.clone(), &snap);
                black_box(p.map(|x| x.kind()));
            }
            BenchMode::FullConsume => {
                if let Some(plan) =
                    dispatch_plan(variant, stream, head.clone(), rest.clone(), &snap)
                {
                    black_box(consume_wire_plan(plan));
                }
            }
        }
    }
    let elapsed = start.elapsed();
    elapsed.as_nanos() as f64 / f64::from(iters)
}

pub fn bench_full_path_variant(
    variant: BenchVariant,
    mode: BenchMode,
    shape: BenchShape,
) -> BenchStats {
    let mut samples = Vec::with_capacity(FULL_PATH_RUNS);
    for run in 0..FULL_PATH_RUNS {
        let ns = run_once_ns(variant, mode, shape, FULL_PATH_ITERS, FULL_PATH_WARMUP);
        samples.push(ns);
        // rotate warmup offset between runs
        let _ = black_box(run);
    }
    BenchStats::from_samples(samples)
}

pub fn run_protector_loop(variant: BenchVariant, iters: u32) -> f64 {
    let shape = BenchShape::P1Static;
    let (head, rest) = shape.head_rest();
    let snap = fixtures::borrowed_snap();
    let warmup = iters / 10;

    for i in 0..warmup {
        let stream = SpikeStream::new(i as u64);
        if let Some(plan) = dispatch_plan(variant, stream, head.clone(), rest.clone(), &snap) {
            let tok = consume_wire_plan(plan);
            black_box(tok);
        }
    }

    let start = Instant::now();
    for i in 0..iters {
        let stream = SpikeStream::new((i as u64) | 0x2_0000);
        if let Some(plan) = dispatch_plan(variant, stream, head.clone(), rest.clone(), &snap) {
            let tok = consume_wire_plan(plan);
            black_box(tok);
        }
    }
    start.elapsed().as_nanos() as f64 / f64::from(iters)
}

pub fn variant_label(v: BenchVariant) -> &'static str {
    match v {
        BenchVariant::M2 => "M2",
        BenchVariant::X2 => "X2",
        BenchVariant::F2 => "F2",
        BenchVariant::N2 => "N2",
    }
}

pub fn mode_label(m: BenchMode) -> &'static str {
    match m {
        BenchMode::PlannerOnly => "planner_only",
        BenchMode::WirePlanConstruct => "wireplan_construct",
        BenchMode::FullConsume => "full_consume",
    }
}

pub fn expected_kind(shape: BenchShape) -> SpikeWirePlanKind {
    match shape {
        BenchShape::P1Static => SpikeWirePlanKind::Static,
        BenchShape::P1Proxy => SpikeWirePlanKind::Proxy,
        BenchShape::HyperFallback => SpikeWirePlanKind::Hyper,
    }
}
