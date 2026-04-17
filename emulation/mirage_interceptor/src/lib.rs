//! `LD_PRELOAD` shim that replaces a process's view of `/dev/kfd` and
//! `/dev/dri/renderD*` with a [`RemoteEmulator`] talking to a Mirage
//! daemon.
//!
//! # Architecture
//!
//! 1. `open`/`openat`/`open64` are exported. Paths matching
//!    [`KFD_PATH`] or a `renderD*` node are redirected: a `memfd` is
//!    allocated to serve as a *cookie* file descriptor so that the
//!    calling program sees an ordinary-looking integer, while the
//!    interceptor tracks the actual device class in a global
//!    registry ([`FD_REGISTRY`]).
//! 2. `close` removes the entry from the registry and forwards to
//!    libc.
//! 3. `ioctl` checks the registry: when the fd is tracked, the raw
//!    ioctl number is decoded via the Linux `_IOC_*` macros. The
//!    appropriate `mirage_schema::amdgpu` enum variant is built from
//!    the C argument buffer, shipped to the remote daemon, and the
//!    response written back. Untracked fds pass through to libc.
//!
//! # Scope
//!
//! The Rust ↔ C marshalling is large (one pair per ioctl). This file
//! ships GET_VERSION as a worked example and a macro-driven skeleton
//! for the rest — adding a new ioctl is one macro invocation.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use libc::{O_CLOEXEC, size_t};

use mirage_remote::RemoteEmulator;
use mirage_schema::amdgpu::{AmdkfdIocGetVersionRequest, HandleKfdIoctl, IoctlCtx};
use mirage_schema::amdgpu_error::AmdgpuError;

/// Path to the KFD char device.
pub const KFD_PATH: &str = "/dev/kfd";

/// Prefix of DRM render node files.
pub const DRI_RENDER_PREFIX: &str = "/dev/dri/renderD";

/// Environment variable naming the daemon socket path. If unset, the
/// interceptor behaves transparently (every call falls through to libc).
pub const MIRAGE_SOCKET_ENV: &str = "MIRAGE_INTERCEPTOR_SOCKET";

// ---------------------------------------------------------------------------
// Global state

static REMOTE: OnceLock<Option<RemoteEmulator>> = OnceLock::new();

fn remote() -> Option<&'static RemoteEmulator> {
    REMOTE
        .get_or_init(|| {
            std::env::var_os(MIRAGE_SOCKET_ENV).map(|s| RemoteEmulator::new(PathBuf::from(s)))
        })
        .as_ref()
}

/// Device class a tracked fd refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Kfd,
    DrmRender,
}

impl DeviceKind {
    pub fn classify(path: &Path) -> Option<Self> {
        let s = path.to_str()?;
        if s == KFD_PATH {
            return Some(Self::Kfd);
        }
        if s.starts_with(DRI_RENDER_PREFIX) {
            return Some(Self::DrmRender);
        }
        None
    }
}

static FD_REGISTRY: OnceLock<Mutex<Vec<(c_int, DeviceKind)>>> = OnceLock::new();

fn registry() -> &'static Mutex<Vec<(c_int, DeviceKind)>> {
    FD_REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

/// Public helper used by tests and out-of-process callers.
pub fn register_fd(fd: c_int, kind: DeviceKind) {
    let mut g = registry().lock().unwrap();
    g.retain(|(f, _)| *f != fd);
    g.push((fd, kind));
}

/// Public helper: look up the device class for a tracked fd.
pub fn lookup_fd(fd: c_int) -> Option<DeviceKind> {
    registry()
        .lock()
        .unwrap()
        .iter()
        .find(|(f, _)| *f == fd)
        .map(|(_, k)| *k)
}

/// Public helper: drop a tracked fd.
pub fn forget_fd(fd: c_int) {
    registry().lock().unwrap().retain(|(f, _)| *f != fd);
}

// ---------------------------------------------------------------------------
// ioctl number decoding (matches <asm-generic/ioctl.h>)

mod ioc {
    pub const NRBITS: u32 = 8;
    pub const TYPEBITS: u32 = 8;
    pub const SIZEBITS: u32 = 14;
    pub const NRSHIFT: u32 = 0;
    pub const TYPESHIFT: u32 = NRSHIFT + NRBITS;
    pub const SIZESHIFT: u32 = TYPESHIFT + TYPEBITS;

    pub const NRMASK: u32 = (1 << NRBITS) - 1;
    pub const TYPEMASK: u32 = (1 << TYPEBITS) - 1;
    pub const SIZEMASK: u32 = (1 << SIZEBITS) - 1;

    pub fn nr(cmd: u32) -> u32 {
        (cmd >> NRSHIFT) & NRMASK
    }
    pub fn ty(cmd: u32) -> u32 {
        (cmd >> TYPESHIFT) & TYPEMASK
    }
    pub fn size(cmd: u32) -> u32 {
        (cmd >> SIZESHIFT) & SIZEMASK
    }
}

const KFD_MAGIC: u32 = b'K' as u32;
const DRM_MAGIC: u32 = b'd' as u32;

// ---------------------------------------------------------------------------
// Dispatch macros
//
// The macro invocation below is the single place where a new
// KFD ioctl gets a real round-trip. Everything else returns ENOSYS.

/// `kfd_dispatch!` expands to a `fn dispatch_kfd(...) -> i32`.
///
/// Each row names a KFD ioctl constant, the C arg layout (`$CArgs`),
/// and two closures: one to build the Rust request from the C args
/// pointer, one to write the Rust response back.
macro_rules! kfd_dispatch {
    ($(
        $name:ident => $c_ty:ty, $variant:ident,
            req = $req_expr:expr,
            write = $write_expr:expr ;
    )*) => {
        fn dispatch_kfd(remote: &RemoteEmulator, cmd: u32, arg: *mut c_void) -> c_int {
            let ctx = current_ctx();
            let nr = ioc::nr(cmd);
            let size = ioc::size(cmd) as usize;
            $(
                if nr == (mirage_schema::amdgpu::$name & 0xff) {
                    // SAFETY: kernel ABI guarantees the buffer at `arg`
                    // is at least the declared size for this nr.
                    if size < core::mem::size_of::<$c_ty>() { return errno_to_rc(libc::EINVAL); }
                    let c_args: &mut $c_ty = unsafe { &mut *(arg as *mut $c_ty) };
                    let req = $req_expr(c_args);
                    match remote.$variant(ctx, req) {
                        Ok(resp) => { $write_expr(c_args, resp); 0 }
                        Err(e) => errno_to_rc(e.errno()),
                    }
                } else
            )*
            { errno_to_rc(AmdgpuError::NoSys.errno()) }
        }
    };
}

#[repr(C)]
struct KfdIocGetVersionArgs {
    major: u32,
    minor: u32,
}

kfd_dispatch! {
    AMDKFD_IOC_GET_VERSION => KfdIocGetVersionArgs, amdkfd_ioc_get_version,
        req = |_a: &mut KfdIocGetVersionArgs| AmdkfdIocGetVersionRequest {},
        write = |a: &mut KfdIocGetVersionArgs, r: mirage_schema::amdgpu::AmdkfdIocGetVersionResponse| {
            a.major = r.major_version;
            a.minor = r.minor_version;
        };
}

fn dispatch_drm(_remote: &RemoteEmulator, _cmd: u32, _arg: *mut c_void) -> c_int {
    // DRM ioctl marshalling to be added per variant — same macro shape
    // as `kfd_dispatch!`, against `libdrm_amdgpu` UAPI structs.
    errno_to_rc(AmdgpuError::NoSys.errno())
}

fn current_ctx() -> IoctlCtx {
    // SAFETY: getpid/gettid are always safe.
    let pid = unsafe { libc::getpid() } as u32;
    let tid = unsafe { libc::syscall(libc::SYS_gettid) } as u32;
    IoctlCtx { pid, tid }
}

fn errno_to_rc(errno: i32) -> c_int {
    // SAFETY: setting the thread-local errno is always sound.
    unsafe {
        *libc::__errno_location() = errno;
    }
    -1
}

// ---------------------------------------------------------------------------
// Public dispatch entry points — callable from tests and from the
// LD_PRELOAD hooks below.

/// Dispatch a tracked ioctl using an explicit [`RemoteEmulator`].
/// Primarily used by tests; the production path is
/// [`dispatch_tracked_ioctl`] which pulls the global from the env.
pub fn dispatch_ioctl_with(
    remote: &RemoteEmulator,
    kind: DeviceKind,
    cmd: u32,
    arg: *mut c_void,
) -> c_int {
    let ty = ioc::ty(cmd);
    match kind {
        DeviceKind::Kfd if ty == KFD_MAGIC => dispatch_kfd(remote, cmd, arg),
        DeviceKind::DrmRender if ty == DRM_MAGIC => dispatch_drm(remote, cmd, arg),
        _ => errno_to_rc(libc::ENOTTY),
    }
}

/// Dispatch a tracked ioctl using the global [`RemoteEmulator`]
/// configured through `MIRAGE_INTERCEPTOR_SOCKET`. Returns `ENOSYS` when
/// the env var is unset.
pub fn dispatch_tracked_ioctl(kind: DeviceKind, cmd: u32, arg: *mut c_void) -> c_int {
    let Some(remote) = remote() else {
        return errno_to_rc(AmdgpuError::NoSys.errno());
    };
    dispatch_ioctl_with(remote, kind, cmd, arg)
}

// ---------------------------------------------------------------------------
// LD_PRELOAD symbol exports
//
// These resolve the next symbol in the dynamic link chain via
// `dlsym(RTLD_NEXT, ...)` so non-GPU paths fall through unchanged.

fn next_symbol(name: &CStr) -> *mut c_void {
    // SAFETY: dlsym with RTLD_NEXT is defined behaviour in glibc.
    unsafe { libc::dlsym(libc::RTLD_NEXT, name.as_ptr()) }
}

macro_rules! next_fn {
    ($name:ident : fn($($arg:ident : $ty:ty),*) -> $ret:ty) => {{
        static SYM: OnceLock<usize> = OnceLock::new();
        let p = *SYM.get_or_init(|| {
            let s = CString::new(stringify!($name)).unwrap();
            next_symbol(&s) as usize
        });
        if p == 0 {
            None
        } else {
            // SAFETY: the resolved symbol matches libc's declared signature.
            let f: unsafe extern "C" fn($($ty),*) -> $ret = unsafe { std::mem::transmute(p) };
            Some(f)
        }
    }};
}

fn cstr_to_path(p: *const c_char) -> Option<PathBuf> {
    if p.is_null() {
        return None;
    }
    // SAFETY: caller guarantees a NUL-terminated string.
    let s = unsafe { CStr::from_ptr(p) };
    Some(PathBuf::from(s.to_str().ok()?))
}

fn create_cookie_fd() -> c_int {
    // Allocate a real fd so that the program sees a normal integer.
    // A `memfd` is a convenient, non-conflicting source.
    let name = CString::new("mirage-cookie").unwrap();
    // SAFETY: memfd_create is a standard glibc call.
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC as u32) };
    fd
}

/// `open(const char *, int, ...)` — variadic in C. We match the two
/// common forms (with and without mode) to avoid the varargs dance.
///
/// # Safety
///
/// Called by the dynamic linker on behalf of user code.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, mode: libc::mode_t) -> c_int {
    if let Some(p) = cstr_to_path(path)
        && let Some(kind) = DeviceKind::classify(&p)
        && remote().is_some()
    {
        let fd = create_cookie_fd();
        if fd >= 0 {
            register_fd(fd, kind);
        }
        return fd;
    }
    let Some(real) = next_fn!(open : fn(p: *const c_char, f: c_int, m: libc::mode_t) -> c_int)
    else {
        return errno_to_rc(libc::ENOSYS);
    };
    // SAFETY: forwarded to libc with identical signature.
    unsafe { real(path, flags | O_CLOEXEC & O_CLOEXEC, mode) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openat(
    dirfd: c_int,
    path: *const c_char,
    flags: c_int,
    mode: libc::mode_t,
) -> c_int {
    if dirfd == libc::AT_FDCWD
        && let Some(p) = cstr_to_path(path)
        && let Some(kind) = DeviceKind::classify(&p)
        && remote().is_some()
    {
        let fd = create_cookie_fd();
        if fd >= 0 {
            register_fd(fd, kind);
        }
        return fd;
    }
    let Some(real) =
        next_fn!(openat : fn(d: c_int, p: *const c_char, f: c_int, m: libc::mode_t) -> c_int)
    else {
        return errno_to_rc(libc::ENOSYS);
    };
    // SAFETY: forwarded to libc.
    unsafe { real(dirfd, path, flags, mode) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn close(fd: c_int) -> c_int {
    if lookup_fd(fd).is_some() {
        forget_fd(fd);
    }
    let Some(real) = next_fn!(close : fn(f: c_int) -> c_int) else {
        return errno_to_rc(libc::ENOSYS);
    };
    // SAFETY: forwarded to libc.
    unsafe { real(fd) }
}

/// `ioctl(int, unsigned long, ...)` — we match the common
/// `(fd, cmd, void *)` shape.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ioctl(fd: c_int, cmd: libc::c_ulong, arg: *mut c_void) -> c_int {
    if let Some(kind) = lookup_fd(fd) {
        return dispatch_tracked_ioctl(kind, cmd as u32, arg);
    }
    let Some(real) = next_fn!(ioctl : fn(f: c_int, c: libc::c_ulong, a: *mut c_void) -> c_int)
    else {
        return errno_to_rc(libc::ENOSYS);
    };
    // SAFETY: forwarded to libc.
    unsafe { real(fd, cmd, arg) }
}

// Silence "unused" for the mode size helper.
#[allow(dead_code)]
fn _mode_size_guard() -> size_t {
    0
}

#[cfg(test)]
mod tests;
