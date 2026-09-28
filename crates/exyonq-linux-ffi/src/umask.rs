//! Process umask restore helper (Unix control-socket bind).

use std::sync::Mutex;

static UMASK_LOCK: Mutex<()> = Mutex::new(());

#[cfg(target_os = "macos")]
type ModeT = u16;
#[cfg(not(target_os = "macos"))]
type ModeT = u32;

unsafe extern "C" {
    fn umask(mask: ModeT) -> ModeT;
}

/// Run `f` with process umask temporarily set to `mask`, then restore.
///
/// Serialized so concurrent binds cannot clobber each other's umask.
pub fn with_umask<R>(mask: u32, f: impl FnOnce() -> R) -> R {
    let _guard = UMASK_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mask_t = mask as ModeT;
    let previous = unsafe {
        // SAFETY: umask is process-global; held under UMASK_LOCK for the critical section.
        umask(mask_t)
    };
    let out = f();
    unsafe {
        // SAFETY: restore previous mask under the same lock.
        umask(previous);
    }
    out
}
