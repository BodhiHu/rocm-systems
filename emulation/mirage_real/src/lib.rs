//! [`RealEmulator`] — the [`Emulator`](mirage_schema::emulator::Emulator)
//! implementation that forwards every ioctl to a host AMD GPU.
//!
//! This crate intentionally holds *real* file descriptors to `/dev/kfd`
//! and the DRM render nodes, and dispatches each typed ioctl request
//! down to the kernel via `libc::ioctl`. A matching `RemoteEmulator`
//! running in an intercepted process therefore sees exactly the
//! behaviour the host kernel would provide.
//!
//! # Scope
//!
//! Every variant of [`AnyKfdIoctlRequest`] and [`AnyDrmIoctlRequest`] is
//! dispatched through a single match inside [`RealEmulator`]. Variants
//! that are not yet wired to a raw kernel ioctl return [`AmdgpuError::NoSys`]
//! so that unsupported paths surface cleanly rather than as memory
//! corruption. [`AMDKFD_IOC_GET_VERSION`] is wired as a worked example.
//!
//! # Availability
//!
//! [`RealEmulator::detect`] returns `None` on machines without a KFD
//! device — tests use this to skip gracefully.

macro_rules! nosys_methods {
    ($(fn $method:ident($request:ty) -> $response:ty;)*) => {
        $(
            fn $method(
                &self,
                _ctx: mirage_schema::amdgpu::IoctlCtx,
                _request: $request,
            ) -> mirage_schema::amdgpu_error::AmdgpuResult<$response> {
                Err(mirage_schema::amdgpu_error::AmdgpuError::NoSys)
            }
        )*
    };
}

mod device;
mod drm;
mod fs;
mod ioctl;
mod kfd;

pub use device::RealEmulator;

#[cfg(test)]
mod tests;
