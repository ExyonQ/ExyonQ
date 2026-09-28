//! Library surface for Miri / unit tests (ADR-045 M13). Binary remains the product entry.

#![allow(dead_code)]

pub mod static_serve;

// Re-export capsule id so Miri --lib can link the crate without running main.
pub use exyonq_linux_ffi::CAPSULE_ID;
