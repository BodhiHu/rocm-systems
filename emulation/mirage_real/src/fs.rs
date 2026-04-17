use std::ffi::CString;

use mirage_schema::amdgpu::IoctlCtx;
use mirage_schema::amdgpu_error::{AmdgpuError, AmdgpuResult};
use mirage_schema::syscalls::{FakeStat, HandleAnyFsSyscalls, HandleFsSyscalls};

use crate::RealEmulator;

// Filesystem-syscall surface: the real host kernel already provides
// these. We simply proxy the request back to libc (for `stat`-family
// and `access`) or report `NoSys` for the ones that only make sense
// against the *virtual* fd table a `RemoteEmulator` server would own.
impl HandleFsSyscalls for RealEmulator {
    nosys_methods!(
        fn syscall_open(mirage_schema::syscalls::SyscallOpenRequest) -> mirage_schema::syscalls::SyscallOpenResponse;
        fn syscall_close(mirage_schema::syscalls::SyscallCloseRequest) -> mirage_schema::syscalls::SyscallCloseResponse;
        fn syscall_readlink_fd(mirage_schema::syscalls::SyscallReadlinkFdRequest) -> mirage_schema::syscalls::SyscallReadlinkFdResponse;
        fn syscall_mmap(mirage_schema::syscalls::SyscallMmapRequest) -> mirage_schema::syscalls::SyscallMmapResponse;
        fn syscall_munmap(mirage_schema::syscalls::SyscallMunmapRequest) -> mirage_schema::syscalls::SyscallMunmapResponse;
        fn syscall_read_device(mirage_schema::syscalls::SyscallReadDeviceRequest) -> mirage_schema::syscalls::SyscallReadDeviceResponse;
        fn syscall_dup(mirage_schema::syscalls::SyscallDupRequest) -> mirage_schema::syscalls::SyscallDupResponse;
        fn syscall_atfork_child(mirage_schema::syscalls::SyscallAtforkChildRequest) -> mirage_schema::syscalls::SyscallAtforkChildResponse;
    );

    fn syscall_stat_device(
        &self,
        _ctx: IoctlCtx,
        request: mirage_schema::syscalls::SyscallStatDeviceRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::SyscallStatDeviceResponse> {
        stat_real_device(&request.path)
            .map(|stat| mirage_schema::syscalls::SyscallStatDeviceResponse { stat })
    }

    fn syscall_access(
        &self,
        _ctx: IoctlCtx,
        request: mirage_schema::syscalls::SyscallAccessRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::SyscallAccessResponse> {
        let c = CString::new(request.path).map_err(|_| AmdgpuError::Invalid)?;
        // SAFETY: `access` takes a NUL-terminated string.
        let rc = unsafe { libc::access(c.as_ptr(), request.mode as i32) };
        Ok(mirage_schema::syscalls::SyscallAccessResponse { allowed: rc == 0 })
    }

    fn syscall_sysfs_read(
        &self,
        _ctx: IoctlCtx,
        request: mirage_schema::syscalls::SyscallSysfsReadRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::SyscallSysfsReadResponse> {
        let mut data = std::fs::read(&request.path).map_err(|error| {
            error
                .raw_os_error()
                .map(AmdgpuError::from_errno)
                .unwrap_or(AmdgpuError::NoEntry)
        })?;
        data.truncate(request.max_size as usize);
        Ok(mirage_schema::syscalls::SyscallSysfsReadResponse { data })
    }
}

impl HandleAnyFsSyscalls for RealEmulator {}

fn stat_real_device(path: &str) -> AmdgpuResult<FakeStat> {
    let c = CString::new(path).map_err(|_| AmdgpuError::Invalid)?;
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `stat(2)` with a NUL-terminated path and valid out pointer.
    let rc = unsafe { libc::stat(c.as_ptr(), &mut st) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error()
            .raw_os_error()
            .map(AmdgpuError::from_errno)
            .unwrap_or(AmdgpuError::Io));
    }
    Ok(FakeStat {
        mode: st.st_mode as u32,
        nlink: st.st_nlink as u32,
        rdev: st.st_rdev as u64,
        size: st.st_size as u64,
    })
}
