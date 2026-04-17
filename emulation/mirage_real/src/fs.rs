use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::{Path, PathBuf};

use mirage_schema::amdgpu::IoctlCtx;
use mirage_schema::amdgpu_error::{AmdgpuError, AmdgpuResult};
use mirage_schema::syscalls::{FakeStat, HandleAnyDeviceSyscalls, HandleDeviceSyscalls};
use mirage_schema::topology::{ProvideTopology, Topology};

use crate::RealEmulator;
use crate::ioctl::from_io_error;

// Filesystem-syscall surface: the real host kernel already provides
// these. We simply proxy the request back to libc (for `stat`-family
// and `access`) or report `NoSys` for the ones that only make sense
// against the *virtual* fd table a `RemoteEmulator` server would own.
impl HandleDeviceSyscalls for RealEmulator {
    fn syscall_open(
        &self,
        _ctx: IoctlCtx,
        request: mirage_schema::syscalls::SyscallOpenRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::SyscallOpenResponse> {
        let path = CString::new(request.path).map_err(|_| AmdgpuError::Invalid)?;
        // SAFETY: `path` is NUL terminated and flags/mode are passed through verbatim.
        let fd = unsafe { libc::open(path.as_ptr(), request.flags as i32, request.mode) };
        if fd < 0 {
            return Err(from_io_error(std::io::Error::last_os_error()));
        }
        Ok(mirage_schema::syscalls::SyscallOpenResponse { virtual_fd: fd })
    }

    fn syscall_close(
        &self,
        _ctx: IoctlCtx,
        request: mirage_schema::syscalls::SyscallCloseRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::SyscallCloseResponse> {
        // SAFETY: closing an integer fd is safe; kernel validates it.
        let rc = unsafe { libc::close(request.virtual_fd) };
        if rc != 0 {
            return Err(from_io_error(std::io::Error::last_os_error()));
        }
        Ok(mirage_schema::syscalls::SyscallCloseResponse {})
    }

    fn syscall_readlink_fd(
        &self,
        _ctx: IoctlCtx,
        request: mirage_schema::syscalls::SyscallReadlinkFdRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::SyscallReadlinkFdResponse> {
        let target = std::fs::read_link(PathBuf::from(format!(
            "/proc/self/fd/{}",
            request.virtual_fd
        )))
        .map_err(from_io_error)?;
        Ok(mirage_schema::syscalls::SyscallReadlinkFdResponse {
            target: target.to_string_lossy().into_owned(),
        })
    }

    fn syscall_mmap(
        &self,
        _ctx: IoctlCtx,
        request: mirage_schema::syscalls::SyscallMmapRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::SyscallMmapResponse> {
        // SAFETY: parameters are forwarded verbatim; kernel validates them.
        let addr = unsafe {
            libc::mmap(
                request.addr_hint as usize as *mut libc::c_void,
                request.length as usize,
                request.prot as i32,
                request.flags as i32,
                request.virtual_fd,
                request.offset as libc::off_t,
            )
        };
        if addr == libc::MAP_FAILED {
            return Err(from_io_error(std::io::Error::last_os_error()));
        }
        Ok(mirage_schema::syscalls::SyscallMmapResponse {
            server_addr: addr as usize as u64,
            mapping_id: addr as usize as u64,
        })
    }

    fn syscall_munmap(
        &self,
        _ctx: IoctlCtx,
        request: mirage_schema::syscalls::SyscallMunmapRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::SyscallMunmapResponse> {
        // SAFETY: kernel validates the pointer/length pair.
        let rc = unsafe {
            libc::munmap(
                request.addr as usize as *mut libc::c_void,
                request.length as usize,
            )
        };
        if rc != 0 {
            return Err(from_io_error(std::io::Error::last_os_error()));
        }
        Ok(mirage_schema::syscalls::SyscallMunmapResponse {})
    }

    fn syscall_read_device(
        &self,
        _ctx: IoctlCtx,
        request: mirage_schema::syscalls::SyscallReadDeviceRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::SyscallReadDeviceResponse> {
        let mut data = vec![0u8; request.count as usize];
        // SAFETY: `data` is a valid writable buffer.
        let rc = unsafe {
            libc::read(
                request.virtual_fd,
                data.as_mut_ptr() as *mut libc::c_void,
                data.len(),
            )
        };
        if rc < 0 {
            return Err(from_io_error(std::io::Error::last_os_error()));
        }
        data.truncate(rc as usize);
        Ok(mirage_schema::syscalls::SyscallReadDeviceResponse { data })
    }

    fn syscall_dup(
        &self,
        _ctx: IoctlCtx,
        _request: mirage_schema::syscalls::SyscallDupRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::SyscallDupResponse> {
        Ok(mirage_schema::syscalls::SyscallDupResponse {})
    }

    fn syscall_atfork_child(
        &self,
        _ctx: IoctlCtx,
        _request: mirage_schema::syscalls::SyscallAtforkChildRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::SyscallAtforkChildResponse> {
        Ok(mirage_schema::syscalls::SyscallAtforkChildResponse {})
    }

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
}

impl HandleAnyDeviceSyscalls for RealEmulator {}

impl ProvideTopology for RealEmulator {
    fn get_topology(&self) -> AmdgpuResult<Topology> {
        let root = Path::new("/sys/class/kfd/kfd/topology");
        let mut files = BTreeMap::new();
        read_topology_recursive(root, root, &mut files);
        Ok(Topology { files })
    }
}

/// Recursively read all files under `dir` into the map, keyed by their
/// path relative to `root`.
fn read_topology_recursive(root: &Path, dir: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() {
            read_topology_recursive(root, &path, files);
        } else if ft.is_file() {
            if let Ok(data) = std::fs::read(&path) {
                if let Ok(rel) = path.strip_prefix(root) {
                    files.insert(rel.to_string_lossy().into_owned(), data);
                }
            }
        }
    }
}

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
