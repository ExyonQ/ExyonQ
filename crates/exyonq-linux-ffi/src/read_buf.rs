//! Tokio `ReadBuf` helper — optional feature `tokio-io` (not used by CFD).

use tokio::io::ReadBuf;

/// Mark `n` bytes of `buf`'s unfilled region as initialized and advance the cursor.
///
/// # Preconditions
/// The previous `poll_read` (or equivalent) must have written exactly `n` bytes into
/// the slice returned by `buf.initialize_unfilled()` / equivalent unfilled region.
pub fn assume_init_advance(buf: &mut ReadBuf<'_>, n: usize) {
    unsafe {
        // SAFETY: caller guarantees `n` bytes were initialized in the unfilled region.
        buf.assume_init(n);
    }
    buf.advance(n);
}
