//! The [`Emulator`] trait — a shared supertrait over every AMD ioctl
//! subsystem the Mirage stack can emulate or forward.
//!
//! An [`Emulator`] is simply a type that knows how to handle both KFD and
//! DRM-AMDGPU ioctls. Every implementation is expected to use interior
//! mutability (all methods take `&self`) so the same value can be shared
//! between threads or exposed behind an `Arc`.
//!
//! Two direct implementors are shipped in sibling crates:
//!
//! * `mirage_real::RealEmulator` — forwards every request to the real
//!   hardware via `/dev/kfd` and the DRM render nodes.
//! * `mirage_remote::RemoteEmulator` — proxies every request over a Unix
//!   socket to a daemon holding any other `Emulator`.

use crate::amdgpu::{HandleAnyDrmIoctl, HandleAnyKfdIoctl, HandleDrmIoctl, HandleKfdIoctl};
use crate::syscalls::{HandleAnyFsSyscalls, HandleFsSyscalls};

/// Combined KFD + DRM + filesystem-syscall emulator.
///
/// Supertraits:
///
/// * [`HandleKfdIoctl`] / [`HandleDrmIoctl`] — the two ioctl
///   subsystems defined via [`ioctl_dsl!`](crate::ioctl_dsl).
/// * [`HandleAnyKfdIoctl`] / [`HandleAnyDrmIoctl`] — dispatch helpers,
///   callable on `&dyn Emulator`.
/// * [`HandleFsSyscalls`] / [`HandleAnyFsSyscalls`] — open/close/stat
///   family and mmap, defined via
///   [`syscall_dsl!`](crate::syscall_dsl).
///
/// Implementors must use interior mutability and be `Send + Sync` so
/// they can be shared across threads.
pub trait Emulator:
    HandleKfdIoctl
    + HandleDrmIoctl
    + HandleAnyKfdIoctl
    + HandleAnyDrmIoctl
    + HandleFsSyscalls
    + HandleAnyFsSyscalls
    + Send
    + Sync
{
}

impl<T> Emulator for T where
    T: HandleKfdIoctl
        + HandleDrmIoctl
        + HandleAnyKfdIoctl
        + HandleAnyDrmIoctl
        + HandleFsSyscalls
        + HandleAnyFsSyscalls
        + Send
        + Sync
{
}
