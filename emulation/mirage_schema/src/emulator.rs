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

/// Combined KFD + DRM ioctl emulator.
///
/// All methods are inherited from [`HandleKfdIoctl`] and [`HandleDrmIoctl`].
/// [`HandleAnyKfdIoctl`] and [`HandleAnyDrmIoctl`] are listed as
/// supertraits so that the dynamic-dispatch helpers (`handle_any_*_ioctl`)
/// are callable on `&dyn Emulator` as well as on concrete types.
///
/// Implementors must use interior mutability and be `Send + Sync` so they
/// can be shared across threads.
pub trait Emulator:
    HandleKfdIoctl + HandleDrmIoctl + HandleAnyKfdIoctl + HandleAnyDrmIoctl + Send + Sync
{
}

impl<T> Emulator for T where
    T: HandleKfdIoctl + HandleDrmIoctl + HandleAnyKfdIoctl + HandleAnyDrmIoctl + Send + Sync
{
}
