//! Non-ioctl POSIX syscalls the mirage interceptor forwards to a
//! daemon, so that an intercepted application sees an entirely
//! daemon-controlled view of the GPU device surface.
//!
//! # What this mirrors
//!
//! The old C++ shim in `projects/mirage/src/shim` interposes the full
//! set of glibc entry points listed below. Each entry here documents:
//!
//! * the matching POSIX / glibc call,
//! * which intercepted paths trigger it (GPU device node,
//!   `/proc/self/fd/N`, …),
//! * what the shim does with the result (e.g. install a memfd-backed
//!   cookie fd, fake a character-device `stat`).
//!
//! Every syscall goes through the [`HandleDeviceSyscalls`] trait exactly
//! the way every ioctl goes through [`HandleKfdIoctl`](crate::amdgpu::HandleKfdIoctl)
//! / [`HandleDrmIoctl`](crate::amdgpu::HandleDrmIoctl). A `Remote*`
//! proxy forwards by implementing [`ForwardDeviceSyscalls`].
//!
//! Topology reads (files under `/sys/class/kfd/kfd/topology/`) are
//! handled separately via [`ProvideTopology`](crate::topology::ProvideTopology).
//!
//! # Paths that drive interposition
//!
//! | Pattern                                      | Device class            |
//! |----------------------------------------------|-------------------------|
//! | `/dev/kfd`                                   | [`DeviceClass::Kfd`]    |
//! | `/dev/dri/renderD<N>`                        | [`DeviceClass::DrmRender`] |
//! | `/dev/dri/card<N>`                           | [`DeviceClass::DrmCard` ] |
//! | `/proc/self/fd/<N>` for a tracked `N`        | handled by readlink     |

use serde::{Deserialize, Serialize};

use crate::syscall_dsl;

/// Classification of an intercepted path. Mirrors the `DeviceType`
/// enum in the old C++ shim, extended with the sysfs topology node so
/// the server can synthesise the kfd topology the kernel would
/// normally expose through sysfs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceClass {
    /// `/dev/kfd` — the Kernel Fusion Driver char device.
    Kfd,
    /// `/dev/dri/renderD<N>` — the AMDGPU DRM render node.
    DrmRender,
    /// `/dev/dri/card<N>` — the AMDGPU DRM primary node.
    DrmCard,
}

/// Subset of `struct stat` fields the shim actually fills. Keeping the
/// wire payload small avoids paying for inode-number / timestamp
/// bits the intercepted application never looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FakeStat {
    /// `st_mode` — always includes `S_IFCHR` for `/dev/kfd|dri/*`
    /// so fstat/stat-family checks in ROCm succeed.
    pub mode: u32,
    /// `st_nlink`.
    pub nlink: u32,
    /// `st_rdev` — major/minor packed the way `makedev(3)` would.
    pub rdev: u64,
    /// `st_size`, used when the path proxies a regular sysfs file.
    pub size: u64,
}

/// `prot` bits a caller is requesting in a memory map (`PROT_READ`,
/// `PROT_WRITE`, `PROT_EXEC`). Kept as a raw `u32` so the server can
/// forward verbatim.
pub type ProtBits = u32;

/// `flags` bits on an `mmap`/`mmap64` call (`MAP_SHARED`, `MAP_FIXED`,
/// …). Kept as raw `u32`.
pub type MmapFlags = u32;

/// `flags` bits on an `open`/`openat` call.
pub type OpenFlags = u32;

syscall_dsl! {
    device {
        /// `open(const char*, int, mode_t)` / `open64` / `openat` — when
        /// the path matches [`DeviceClass::Kfd`], [`DeviceClass::DrmRender`],
        /// or [`DeviceClass::DrmCard`]. The shim allocates a local
        /// `memfd` as a *cookie fd* (so the process sees a normal
        /// integer), registers it against the returned `virtual_fd`,
        /// and subsequent `ioctl`/`mmap`/`close` calls on that fd
        /// route back here. Mirrors `do_proxy_open` in the C++ shim.
        SyscallOpen {
            path : String,
            flags : OpenFlags,
            mode : u32,
            class : DeviceClass,
        } => {
            virtual_fd : i32,
        };

        /// `close(int)` — forwarded so the server can drop its
        /// bookkeeping for the `virtual_fd`. Mirrors the `CloseRequest`
        /// branch of the shim's `close` interposer.
        SyscallClose {
            virtual_fd : i32,
        } => {};

        /// `stat(path)` / `fstatat(..., path, ...)` / `__xstat(path)` —
        /// when the path is a GPU device node the shim fills a
        /// synthesised character-device stat. Mirrors
        /// `fill_fake_device_stat` / `fill_fake_device_stat64`.
        ///
        /// Returned in [`SyscallStatDeviceResponse::stat`].
        SyscallStatDevice {
            class : DeviceClass,
            path : String,
        } => {
            stat : FakeStat,
        };

        /// `access(path, mode)` / `faccessat(..., path, mode, ...)` —
        /// always succeeds for GPU device paths in the shim; the
        /// server still gets asked so it can return `EACCES` when the
        /// caller does not own the virtualised device.
        SyscallAccess {
            class : DeviceClass,
            path : String,
            mode : u32,
        } => {
            allowed : bool,
        };

        /// `readlink("/proc/self/fd/<N>", ...)` for a tracked `N` —
        /// the shim responds with the *original* GPU device path so
        /// downstream code that sniffs `/proc/self/fd` (as ROCr does)
        /// sees `/dev/kfd` or `/dev/dri/renderD128` even though the
        /// actual kernel fd is a memfd.
        SyscallReadlinkFd {
            virtual_fd : i32,
        } => {
            /// Target path (e.g. `"/dev/kfd"`).
            target : String,
        };

        /// `mmap`/`mmap64` on a tracked GPU fd. The shim sends the
        /// request to the server, which allocates a shared-memory
        /// region (dma-buf or memfd) and returns its metadata plus,
        /// in the real C++ shim, an fd over `SCM_RIGHTS`. The
        /// serde_bare wire format used here does not support fd
        /// passing, so an emulator that needs a real shared mapping
        /// must wrap this response in an out-of-band fd transfer
        /// (see `mirage_remote::protocol::WireRequest::MmapFd`).
        ///
        /// Mirrors `proxy_mmap` in `shim.cpp`.
        SyscallMmap {
            virtual_fd : i32,
            addr_hint : u64,
            length : u64,
            offset : u64,
            prot : ProtBits,
            flags : MmapFlags,
        } => {
            /// Server-chosen mapping address when non-zero (mostly
            /// 0 — the shim maps the received fd with `MAP_FIXED` at
            /// `addr_hint` to make it visible to the client).
            server_addr : u64,
            /// Opaque cookie the server uses to correlate a later
            /// `Munmap`. 0 if the server does not track mappings.
            mapping_id : u64,
        };

        /// `munmap(addr, length)` on a region previously returned by
        /// [`SyscallMmap`]. The shim sends this after `real_munmap`
        /// unmaps the region locally, so the server can release any
        /// backing resources. Mirrors the `munmap` interposer in
        /// `shim.cpp`.
        SyscallMunmap {
            mapping_id : u64,
            addr : u64,
            length : u64,
        } => {};

        /// `read(fd, buf, count)` on a tracked GPU fd. DRM event
        /// files return an empty event stream in the simulator; sysfs
        /// reads go through [`ProvideTopology`](crate::topology::ProvideTopology)
        /// and never reach this path. Mirrors the `read` interposer.
        SyscallReadDevice {
            virtual_fd : i32,
            count : u64,
        } => {
            data : Vec<u8>,
        };

        /// `dup(old)` / `dup2(old, new)` / `dup3(old, new, flags)` /
        /// `fcntl(old, F_DUPFD[_CLOEXEC])` — lets the server keep a
        /// reference count of how many process-side fds alias a
        /// single `virtual_fd`, so it does not tear the state down
        /// while any alias remains open. Mirrors the C++ shim's
        /// `FdTable::Dup`.
        SyscallDup {
            virtual_fd : i32,
        } => {};

        /// `pthread_atfork` child hook — invoked by the shim after
        /// `fork()` to let the server invalidate inherited fds so
        /// the child starts from a clean slate. Mirrors
        /// `FdTable::ClearForFork`.
        SyscallAtforkChild {
            pid : u32,
        } => {};
    }
}
