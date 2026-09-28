//! AUDITOR must-pass: rustdoc describing intentional empty default hooks.
//!
//! /// Default: no-op until a production runtime overrides.
pub trait DrainHooks {
    fn begin_drain(&self) {}
}
