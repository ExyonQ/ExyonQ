//! INTERNAL — Linux FFI capsule (ADR-045). Tokio-free by default.
//! NOT STABLE PUBLIC API. Sole intended home for product-tree `unsafe` syscall wrappers
//! behind Safe APIs (CFD + mod-static + core share this crate).

// ADR-045 capsule: this crate is the allowlisted home for Linux syscall `unsafe`.

#[cfg(target_os = "linux")]
mod fd;
#[cfg(unix)]
mod flock;
#[cfg(unix)]
mod linkat;
#[cfg(target_os = "linux")]
mod net;
#[cfg(target_os = "linux")]
mod openat2;
#[cfg(target_os = "linux")]
mod sendfile;
#[cfg(unix)]
mod umask;
#[cfg(unix)]
mod unix_fs;

#[cfg(feature = "tokio-io")]
mod read_buf;

#[cfg(target_os = "linux")]
pub use fd::{close_raw_fd, dup_raw_fd};
#[cfg(unix)]
pub use flock::try_flock_exclusive;
#[cfg(unix)]
pub use linkat::hard_link_at;
#[cfg(target_os = "linux")]
pub use net::{send_all_nosignal, send_vectored_nosignal, set_abortive_linger, write_all_fd};
#[cfg(target_os = "linux")]
pub use openat2::{
    openat2_beneath, openat2_rdonly_beneath, openat_directory_nofollow, openat_readonly_nofollow,
    Openat2Error,
};
#[cfg(target_os = "linux")]
pub use sendfile::{sendfile64_step, SendfileStep};
#[cfg(unix)]
pub use umask::with_umask;
#[cfg(unix)]
pub use unix_fs::{
    create_directory_at, create_file_exclusive_at, effective_uid, metadata_at_nofollow,
    open_directory_at, open_or_create_lease_at, rename_exchange_at, rename_noreplace_at,
    set_fd_mode, unlink_directory_at, unlink_file_at, EntryMetadata,
};

#[cfg(feature = "tokio-io")]
pub use read_buf::assume_init_advance;

/// Capsule identity for composition / gates.
pub const CAPSULE_ID: &str = "exyonq-linux-ffi:adr-045";
