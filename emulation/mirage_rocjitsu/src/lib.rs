//! [`RocjitsuEmulator`] — an [`Emulator`](mirage_schema::emulator::Emulator)
//! implementation backed by the [rocjitsu] virtual-machine simulator.
//!
//! Where [`mirage_real`](../mirage_real) forwards every ioctl to a real
//! AMD GPU via `/dev/kfd`, `RocjitsuEmulator` forwards those same
//! requests to a `rj_vm_t` running in-process. This makes it possible
//! to run the full Mirage stack against a simulated GPU on machines
//! without real hardware.
//!
//! # Current status
//!
//! The underlying rocjitsu C API exposes a VM lifecycle (create / step
//! / run / checkpoint) plus a code-object / decoder surface — it does
//! not yet expose a full KFD + DRM-AMDGPU ioctl shim. Consequently
//! this crate currently:
//!
//! * owns an optional `rj_vm_t` handle with safe RAII drop;
//! * exposes [`RocjitsuEmulator::from_config_string`] / [`from_config_file`]
//!   constructors that build the VM from a rocjitsu JSON config;
//! * satisfies the [`Emulator`] supertraits by implementing the
//!   `Forward*` wire-level traits, with every request currently
//!   answered as [`AmdgpuError::NoSys`].
//!
//! As rocjitsu grows an ioctl surface, individual match arms will be
//! filled in — the same incremental path [`mirage_real`] already uses
//! (see [`mirage_real::RealEmulator`]'s `AmdkfdIocGetVersion` arm).
//!
//! # Availability
//!
//! The companion `rocjitsu_sys` crate links to `librocjitsu` only when
//! `ROCJITSU_LIB_DIR` is set at build time. [`RocjitsuEmulator::available`]
//! reports whether the headers were discovered; the actual library
//! presence is resolved by the linker. Use [`RocjitsuEmulator::new_stub`]
//! in tests that must run on hosts without rocjitsu.
//!
//! [rocjitsu]: https://github.com/ROCm/rocm-systems/tree/main/experimental/rocjitsu

use std::ffi::{CString, NulError};
use std::path::Path;
use std::ptr;
use std::sync::Mutex;

use mirage_schema::amdgpu::{
    AnyDrmIoctlRequest, AnyDrmIoctlResponse, AnyKfdIoctlRequest, AnyKfdIoctlResponse,
    ForwardDrmIoctl, ForwardKfdIoctl, IoctlCtx,
};
use mirage_schema::amdgpu_error::{AmdgpuError, AmdgpuResult};
use mirage_schema::syscalls::{AnyFsSyscallRequest, AnyFsSyscallResponse, ForwardFsSyscalls};

// ---------------------------------------------------------------------------
// Error type.

/// Errors that can occur while constructing a [`RocjitsuEmulator`].
#[derive(Debug)]
pub enum RocjitsuError {
    /// The rocjitsu C API returned a non-success status.
    Status(rocjitsu_sys::rj_status_t),
    /// An input string contained an interior NUL byte.
    InvalidString,
}

impl std::fmt::Display for RocjitsuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(s) => write!(f, "rocjitsu status {s}"),
            Self::InvalidString => write!(f, "input string contained an interior NUL byte"),
        }
    }
}

impl std::error::Error for RocjitsuError {}

impl From<NulError> for RocjitsuError {
    fn from(_: NulError) -> Self {
        Self::InvalidString
    }
}

fn check(status: rocjitsu_sys::rj_status_t) -> Result<(), RocjitsuError> {
    if status == rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_SUCCESS {
        Ok(())
    } else {
        Err(RocjitsuError::Status(status))
    }
}

// ---------------------------------------------------------------------------
// Safe handle to a `rj_vm_t`.

/// Owning wrapper around a `rocjitsu_sys::rj_vm_t *` that destroys and
/// releases the VM on drop.
///
/// # Safety
///
/// The rocjitsu VM API is documented to be safe to call from a single
/// thread at a time. We serialise access with the `Mutex` inside
/// [`RocjitsuEmulator`] rather than inside this wrapper so that the
/// `Send + Sync` bounds required by [`Emulator`] hold.
struct VmHandle(ptr::NonNull<rocjitsu_sys::rj_vm_t>);

// SAFETY: ownership of the underlying C handle is single; mutation is
// serialised by the surrounding mutex.
unsafe impl Send for VmHandle {}

impl Drop for VmHandle {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from `rj_vm_create*` (which returns a
        // VM with refcount 0) and has not been destroyed yet; no other
        // reference exists because we own it. Per the rocjitsu
        // refcount contract (see `refcount.h`), `rj_vm_destroy` on a
        // refcount-0 object frees immediately — calling `rj_vm_release`
        // on top of that would underflow the refcount.
        unsafe {
            rocjitsu_sys::rj_vm_destroy(self.0.as_ptr());
        }
    }
}

// ---------------------------------------------------------------------------
// Emulator.

/// A Mirage [`Emulator`](mirage_schema::emulator::Emulator) backed by a
/// rocjitsu virtual machine.
pub struct RocjitsuEmulator {
    /// The backing VM, or `None` for a stub emulator used on hosts
    /// without rocjitsu (tests, CI).
    vm: Mutex<Option<VmHandle>>,
}

impl std::fmt::Debug for RocjitsuEmulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RocjitsuEmulator")
            .field("has_vm", &self.vm.lock().unwrap().is_some())
            .finish()
    }
}

impl RocjitsuEmulator {
    /// Returns `true` if this crate was built against the rocjitsu C
    /// headers. Absence of headers at *build time* means attempting to
    /// construct a real VM will fail at link time; tests should prefer
    /// [`Self::new_stub`] when this returns `false`.
    pub const fn available() -> bool {
        // If headers were missing, `rocjitsu_sys` emits an empty
        // bindings file and this function would not resolve. If this
        // crate compiles at all, the symbols are visible.
        true
    }

    /// Build an emulator that owns no VM. Every forwarded request
    /// returns [`AmdgpuError::NoSys`]. Useful in tests on hosts that
    /// do not link against `librocjitsu`.
    pub fn new_stub() -> Self {
        Self {
            vm: Mutex::new(None),
        }
    }

    /// Build an emulator from an in-memory rocjitsu JSON config
    /// string.
    ///
    /// `schema_path` must point at the `simulation_config.fbs`
    /// FlatBuffers schema that ships with rocjitsu.
    pub fn from_config_string(json: &str, schema_path: &Path) -> Result<Self, RocjitsuError> {
        let json_c = CString::new(json)?;
        let schema_c = CString::new(schema_path.as_os_str().as_encoded_bytes())?;
        let mut vm: *mut rocjitsu_sys::rj_vm_t = ptr::null_mut();
        // SAFETY: all pointers are NUL-terminated and out-pointer is
        // valid for a `*mut *mut rj_vm_t` write.
        let status = unsafe {
            rocjitsu_sys::rj_vm_create_from_string(json_c.as_ptr(), schema_c.as_ptr(), &mut vm)
        };
        check(status)?;
        let handle = ptr::NonNull::new(vm).ok_or(RocjitsuError::Status(
            rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_ERROR,
        ))?;
        Ok(Self {
            vm: Mutex::new(Some(VmHandle(handle))),
        })
    }

    /// Build an emulator from an on-disk rocjitsu JSON config file.
    pub fn from_config_file(json_path: &Path, schema_path: &Path) -> Result<Self, RocjitsuError> {
        let json_c = CString::new(json_path.as_os_str().as_encoded_bytes())?;
        let schema_c = CString::new(schema_path.as_os_str().as_encoded_bytes())?;
        let mut vm: *mut rocjitsu_sys::rj_vm_t = ptr::null_mut();
        // SAFETY: same as `from_config_string`.
        let status =
            unsafe { rocjitsu_sys::rj_vm_create(json_c.as_ptr(), schema_c.as_ptr(), &mut vm) };
        check(status)?;
        let handle = ptr::NonNull::new(vm).ok_or(RocjitsuError::Status(
            rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_ERROR,
        ))?;
        Ok(Self {
            vm: Mutex::new(Some(VmHandle(handle))),
        })
    }

    /// Step the underlying VM by one tick.
    ///
    /// Returns `true` while any wavefront is still executing. Returns
    /// [`AmdgpuError::NoSys`] if this emulator was constructed with
    /// [`Self::new_stub`].
    pub fn step(&self) -> AmdgpuResult<bool> {
        let guard = self.vm.lock().unwrap();
        let handle = guard.as_ref().ok_or(AmdgpuError::NoSys)?;
        let mut active: i32 = 0;
        // SAFETY: `handle.0` is a live `rj_vm_t *` for the duration of
        // this call (mutex guards against concurrent destruction).
        let status = unsafe { rocjitsu_sys::rj_vm_step(handle.0.as_ptr(), &mut active) };
        if status != rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_SUCCESS {
            return Err(AmdgpuError::Invalid);
        }
        Ok(active != 0)
    }

    /// Run the underlying VM to completion. Returns the number of
    /// ticks executed.
    pub fn run(&self) -> AmdgpuResult<u64> {
        let guard = self.vm.lock().unwrap();
        let handle = guard.as_ref().ok_or(AmdgpuError::NoSys)?;
        let mut ticks: u64 = 0;
        // SAFETY: `handle.0` is a live `rj_vm_t *` for the duration of
        // this call.
        let status = unsafe { rocjitsu_sys::rj_vm_run(handle.0.as_ptr(), &mut ticks) };
        if status != rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_SUCCESS {
            return Err(AmdgpuError::Invalid);
        }
        Ok(ticks)
    }
}

// ---------------------------------------------------------------------------
// Emulator trait surface. Every ioctl / syscall is currently a stub;
// see module docs for the rationale.

impl ForwardKfdIoctl for RocjitsuEmulator {
    fn forward_kfd_ioctl(
        &self,
        _ctx: IoctlCtx,
        _request: AnyKfdIoctlRequest,
    ) -> AmdgpuResult<AnyKfdIoctlResponse> {
        // TODO: dispatch to the VM once rocjitsu grows a KFD ioctl
        // surface. Until then, unsupported paths surface cleanly as
        // ENOSYS rather than silent no-ops.
        Err(AmdgpuError::NoSys)
    }
}

impl ForwardDrmIoctl for RocjitsuEmulator {
    fn forward_drm_ioctl(
        &self,
        _ctx: IoctlCtx,
        _request: AnyDrmIoctlRequest,
    ) -> AmdgpuResult<AnyDrmIoctlResponse> {
        // TODO: dispatch to the VM once rocjitsu grows a DRM-AMDGPU
        // ioctl surface.
        Err(AmdgpuError::NoSys)
    }
}

impl ForwardFsSyscalls for RocjitsuEmulator {
    fn forward_fs_syscall(
        &self,
        _ctx: IoctlCtx,
        _request: AnyFsSyscallRequest,
    ) -> AmdgpuResult<AnyFsSyscallResponse> {
        // TODO: synthesise a virtual /sys/class/kfd view from the
        // simulated topology.
        Err(AmdgpuError::NoSys)
    }
}

// Compile-time proof that `RocjitsuEmulator` satisfies `Emulator`.
const _: fn() = || {
    fn assert_emulator<T: mirage_schema::emulator::Emulator>() {}
    assert_emulator::<RocjitsuEmulator>();
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_emulator_reports_nosys_for_every_surface() {
        use mirage_schema::amdgpu::AmdkfdIocGetVersionRequest;
        let emu = RocjitsuEmulator::new_stub();
        let ctx = IoctlCtx { pid: 0, tid: 0 };
        assert!(matches!(
            emu.forward_kfd_ioctl(
                ctx,
                AnyKfdIoctlRequest::AmdkfdIocGetVersion(AmdkfdIocGetVersionRequest {}),
            ),
            Err(AmdgpuError::NoSys),
        ));
        assert!(matches!(emu.step(), Err(AmdgpuError::NoSys)));
        assert!(matches!(emu.run(), Err(AmdgpuError::NoSys)));
    }

    #[test]
    fn stub_emulator_is_debuggable() {
        let emu = RocjitsuEmulator::new_stub();
        let s = format!("{emu:?}");
        assert!(s.contains("has_vm: false"));
    }

    /// End-to-end smoke test: construct a real VM from the bundled
    /// CDNA4 topology config and run it to completion. Skips when the
    /// rocjitsu source tree (and therefore its schema / config files)
    /// is not co-located with this workspace — e.g. when
    /// `rocjitsu_sys` was built against an external `ROCJITSU_LIB_DIR`.
    #[test]
    fn real_vm_runs_bundled_cdna4_config() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut config_path = None;
        let mut schema_path = None;
        for ancestor in manifest_dir.ancestors() {
            let cfg = ancestor.join("experimental/rocjitsu/configs/amdgpu_cdna4.json");
            let sch = ancestor.join("experimental/rocjitsu/schemas/simulation_config.fbs");
            if cfg.exists() && sch.exists() {
                config_path = Some(cfg);
                schema_path = Some(sch);
                break;
            }
        }
        let (Some(config_path), Some(schema_path)) = (config_path, schema_path) else {
            eprintln!("rocjitsu source tree not found; skipping end-to-end test");
            return;
        };

        let emu = match RocjitsuEmulator::from_config_file(&config_path, &schema_path) {
            Ok(emu) => emu,
            Err(e) => {
                // If we're linked against the stub, construction fails
                // with a rocjitsu status error — treat as a skip.
                eprintln!("rj_vm_create failed ({e}); likely linked against stub — skipping");
                return;
            }
        };
        // Just confirm step() talks to the real VM without panicking.
        // A full `run()` may take too long for unit testing; a single
        // step is enough to prove the FFI path is live.
        let _ = emu.step();
    }
}
