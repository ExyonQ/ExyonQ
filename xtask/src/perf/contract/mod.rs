//! Plan 03 — performance contract (baseline compare gate).
//!
//! Harness helpers and schema types are consumed by shell wrappers, fixtures, and
//! unit tests — not every symbol is referenced from `main.rs`.

#![allow(dead_code, clippy::too_many_arguments)]

mod compare;
mod normalize;
mod registry;
mod run;
mod scenarios;
mod schema;

pub use run::{contract_command, ContractArgs};
