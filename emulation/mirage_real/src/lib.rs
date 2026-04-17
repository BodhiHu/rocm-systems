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
//! Every schema-defined KFD, DRM, and filesystem entrypoint in
//! [`RealEmulator`] is translated into the corresponding host kernel ABI
//! and forwarded directly with `libc::ioctl` or the matching libc syscall.
//! Variable-length payloads are marshalled into the exact C layout expected
//! by the driver before submission.
//!
//! # Availability
//!
//! [`RealEmulator::detect`] returns `None` on machines without a KFD
//! device — tests use this to skip gracefully.

mod device;
mod drm;
mod fs;
mod ioctl;
mod kfd;

pub use device::RealEmulator;

#[cfg(test)]
mod tests;
