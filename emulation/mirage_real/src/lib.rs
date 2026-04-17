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

use std::fs::OpenOptions;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use mirage_schema::amdgpu::{
    AmdkfdIocGetVersionResponse, AnyDrmIoctlRequest, AnyDrmIoctlResponse, AnyKfdIoctlRequest,
    AnyKfdIoctlResponse, ForwardDrmIoctl, ForwardKfdIoctl, IoctlCtx,
};
use mirage_schema::amdgpu_error::{AmdgpuError, AmdgpuResult};

const KFD_DEVICE_PATH: &str = "/dev/kfd";
const DRI_DIR: &str = "/dev/dri";

/// Linux `_IO` direction bits (same layout as `<asm-generic/ioctl.h>`).
mod ioc {
    pub const NRBITS: u32 = 8;
    pub const TYPEBITS: u32 = 8;
    pub const SIZEBITS: u32 = 14;

    pub const NRSHIFT: u32 = 0;
    pub const TYPESHIFT: u32 = NRSHIFT + NRBITS;
    pub const SIZESHIFT: u32 = TYPESHIFT + TYPEBITS;
    pub const DIRSHIFT: u32 = SIZESHIFT + SIZEBITS;

    #[allow(dead_code)]
    pub const NONE: u32 = 0;
    pub const WRITE: u32 = 1;
    pub const READ: u32 = 2;

    pub const fn ioc(dir: u32, ty: u32, nr: u32, size: u32) -> u32 {
        (dir << DIRSHIFT) | (ty << TYPESHIFT) | (nr << NRSHIFT) | (size << SIZESHIFT)
    }

    pub const fn iowr(ty: u32, nr: u32, size: u32) -> u32 {
        ioc(READ | WRITE, ty, nr, size)
    }
}

/// KFD char device ioctl type.
const KFDIOC_MAGIC: u32 = b'K' as u32;

/// Handle to the real hardware.
pub struct RealEmulator {
    kfd: OwnedFd,
    /// DRM render nodes, keyed by minor number so the GPU index is stable
    /// across calls. Lazily populated on first access. A `Mutex` is fine
    /// here — these are not hot paths compared to ioctl round-trips.
    render_nodes: Mutex<Vec<RenderNode>>,
}

struct RenderNode {
    path: PathBuf,
    #[allow(dead_code)] // held open to keep the kernel-side fd alive
    fd: OwnedFd,
}

impl std::fmt::Debug for RealEmulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RealEmulator")
            .field("kfd_fd", &self.kfd.as_raw_fd())
            .finish_non_exhaustive()
    }
}

impl RealEmulator {
    /// Return `true` if this host has a KFD device node at all, meaning
    /// [`RealEmulator::detect`] has a chance of succeeding.
    pub fn hardware_available() -> bool {
        Path::new(KFD_DEVICE_PATH).exists()
    }

    /// Open `/dev/kfd` and enumerate `/dev/dri/renderD*`. Returns `None`
    /// if no KFD device is present, otherwise propagates the underlying
    /// `io::Error`.
    pub fn detect() -> io::Result<Option<Self>> {
        if !Self::hardware_available() {
            return Ok(None);
        }
        let kfd = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(KFD_DEVICE_PATH)?;
        let render_nodes = Self::open_render_nodes();
        Ok(Some(Self {
            kfd: kfd.into(),
            render_nodes: Mutex::new(render_nodes),
        }))
    }

    fn open_render_nodes() -> Vec<RenderNode> {
        let mut nodes = Vec::new();
        let Ok(dir) = std::fs::read_dir(DRI_DIR) else {
            return nodes;
        };
        let mut entries: Vec<_> = dir
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("renderD"))
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if let Ok(fd) = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_CLOEXEC)
                .open(entry.path())
            {
                nodes.push(RenderNode {
                    path: entry.path(),
                    fd: fd.into(),
                });
            }
        }
        nodes
    }

    /// List of DRM render-node paths successfully opened.
    pub fn render_node_paths(&self) -> Vec<PathBuf> {
        self.render_nodes
            .lock()
            .unwrap()
            .iter()
            .map(|n| n.path.clone())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Raw ioctl helpers.

/// `ioctl(fd, nr, &mut arg)` — returns the kernel errno on failure.
///
/// # Safety
///
/// Caller must ensure that `arg` is a valid pointer of the exact C
/// layout expected by the driver for ioctl number `nr`.
unsafe fn raw_ioctl<T>(fd: i32, nr: u32, arg: &mut T) -> AmdgpuResult<()> {
    // SAFETY: caller invariants.
    let rc = unsafe { libc::ioctl(fd, nr as _, arg as *mut T) };
    if rc == 0 {
        Ok(())
    } else {
        let err = io::Error::last_os_error();
        Err(err
            .raw_os_error()
            .map(AmdgpuError::from_errno)
            .unwrap_or(AmdgpuError::Io))
    }
}

// --- KFD: GET_VERSION -------------------------------------------------------

#[repr(C)]
struct KfdIocGetVersionArgs {
    major: u32,
    minor: u32,
}

impl RealEmulator {
    fn kfd_get_version(&self) -> AmdgpuResult<AmdkfdIocGetVersionResponse> {
        let nr = ioc::iowr(
            KFDIOC_MAGIC,
            0x01,
            core::mem::size_of::<KfdIocGetVersionArgs>() as u32,
        );
        let mut args = KfdIocGetVersionArgs { major: 0, minor: 0 };
        // SAFETY: `args` has the exact layout the kernel expects.
        unsafe { raw_ioctl(self.kfd.as_raw_fd(), nr, &mut args)? };
        Ok(AmdkfdIocGetVersionResponse {
            major_version: args.major,
            minor_version: args.minor,
        })
    }
}

// ---------------------------------------------------------------------------
// Forwarding impls.

impl ForwardKfdIoctl for RealEmulator {
    fn forward_kfd_ioctl(
        &self,
        _ctx: IoctlCtx,
        request: AnyKfdIoctlRequest,
    ) -> AmdgpuResult<AnyKfdIoctlResponse> {
        match request {
            AnyKfdIoctlRequest::AmdkfdIocGetVersion(_) => self
                .kfd_get_version()
                .map(AnyKfdIoctlResponse::AmdkfdIocGetVersion),
            // TODO: remaining KFD ioctls require per-variant C-struct
            // marshalling against the libhsakmt UAPI. They are stubbed
            // as `ENOSYS` so that unsupported paths surface cleanly.
            _ => Err(AmdgpuError::NoSys),
        }
    }
}

impl ForwardDrmIoctl for RealEmulator {
    fn forward_drm_ioctl(
        &self,
        _ctx: IoctlCtx,
        _request: AnyDrmIoctlRequest,
    ) -> AmdgpuResult<AnyDrmIoctlResponse> {
        // TODO: DRM-AMDGPU ioctls require per-variant C-struct marshalling
        // against the libdrm_amdgpu UAPI plus per-GPU fd dispatch.
        Err(AmdgpuError::NoSys)
    }
}

// Filesystem-syscall surface: the real host kernel already provides
// these. We simply proxy the request back to libc (for `stat`-family
// and `access`) or report `NoSys` for the ones that only make sense
// against the *virtual* fd table a `RemoteEmulator` server would own.
impl mirage_schema::syscalls::ForwardFsSyscalls for RealEmulator {
    fn forward_fs_syscall(
        &self,
        _ctx: IoctlCtx,
        request: mirage_schema::syscalls::AnyFsSyscallRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::AnyFsSyscallResponse> {
        use mirage_schema::syscalls::*;
        match request {
            AnyFsSyscallRequest::SyscallStatDevice(SyscallStatDeviceRequest { path, .. }) => {
                stat_real_device(&path).map(|stat| {
                    AnyFsSyscallResponse::SyscallStatDevice(SyscallStatDeviceResponse { stat })
                })
            }
            AnyFsSyscallRequest::SyscallAccess(SyscallAccessRequest { path, mode, .. }) => {
                let c = std::ffi::CString::new(path).map_err(|_| AmdgpuError::Invalid)?;
                // SAFETY: `access` takes a NUL-terminated string.
                let rc = unsafe { libc::access(c.as_ptr(), mode as i32) };
                Ok(AnyFsSyscallResponse::SyscallAccess(SyscallAccessResponse {
                    allowed: rc == 0,
                }))
            }
            AnyFsSyscallRequest::SyscallSysfsRead(SyscallSysfsReadRequest { path, max_size }) => {
                let mut data = std::fs::read(&path).map_err(|e| {
                    e.raw_os_error()
                        .map(AmdgpuError::from_errno)
                        .unwrap_or(AmdgpuError::NoEntry)
                })?;
                data.truncate(max_size as usize);
                Ok(AnyFsSyscallResponse::SyscallSysfsRead(
                    SyscallSysfsReadResponse { data },
                ))
            }
            _ => Err(AmdgpuError::NoSys),
        }
    }
}

fn stat_real_device(path: &str) -> AmdgpuResult<mirage_schema::syscalls::FakeStat> {
    let c = std::ffi::CString::new(path).map_err(|_| AmdgpuError::Invalid)?;
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `stat(2)` with a NUL-terminated path and valid out pointer.
    let rc = unsafe { libc::stat(c.as_ptr(), &mut st) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error()
            .raw_os_error()
            .map(AmdgpuError::from_errno)
            .unwrap_or(AmdgpuError::Io));
    }
    Ok(mirage_schema::syscalls::FakeStat {
        mode: st.st_mode as u32,
        nlink: st.st_nlink as u32,
        rdev: st.st_rdev as u64,
        size: st.st_size as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_hardware_presence_without_panicking() {
        // Whatever the answer, this must not crash.
        let _ = RealEmulator::hardware_available();
    }

    #[test]
    fn detect_returns_none_without_kfd() {
        if RealEmulator::hardware_available() {
            // skip on hosts with real hardware
            return;
        }
        assert!(RealEmulator::detect().unwrap().is_none());
    }

    #[test]
    fn get_version_succeeds_when_hardware_present() {
        let Some(emu) = RealEmulator::detect().unwrap() else {
            eprintln!("no /dev/kfd; skipping");
            return;
        };
        let resp = emu.kfd_get_version().expect("get_version on real hw");
        assert!(resp.major_version >= 1, "kfd reports version {resp:?}");
    }
}
