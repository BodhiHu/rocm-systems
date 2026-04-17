use std::io;
use std::os::fd::AsRawFd;

use mirage_schema::amdgpu::AmdkfdIocGetVersionResponse;
use mirage_schema::amdgpu_error::{AmdgpuError, AmdgpuResult};
use mirage_uapi::kfd;

use crate::RealEmulator;

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

pub(crate) fn from_io_error(error: io::Error) -> AmdgpuError {
    error
        .raw_os_error()
        .map(AmdgpuError::from_errno)
        .unwrap_or(AmdgpuError::Io)
}

pub(crate) fn maybe_ptr<T>(slice: &[T]) -> u64 {
    if slice.is_empty() {
        0
    } else {
        slice.as_ptr() as usize as u64
    }
}

pub(crate) fn maybe_mut_ptr<T>(slice: &mut [T]) -> u64 {
    if slice.is_empty() {
        0
    } else {
        slice.as_mut_ptr() as usize as u64
    }
}

pub(crate) const fn kfd_ior<T>(nr: u32) -> u32 {
    mirage_uapi::ioc::ior(
        mirage_uapi::ioc::KFD_MAGIC,
        nr,
        core::mem::size_of::<T>() as u32,
    )
}

pub(crate) const fn kfd_iow<T>(nr: u32) -> u32 {
    mirage_uapi::ioc::iow(
        mirage_uapi::ioc::KFD_MAGIC,
        nr,
        core::mem::size_of::<T>() as u32,
    )
}

pub(crate) const fn kfd_iowr<T>(nr: u32) -> u32 {
    mirage_uapi::ioc::iowr(
        mirage_uapi::ioc::KFD_MAGIC,
        nr,
        core::mem::size_of::<T>() as u32,
    )
}

pub(crate) const fn drm_iow<T>(nr: u32) -> u32 {
    mirage_uapi::ioc::iow(
        mirage_uapi::ioc::DRM_MAGIC,
        mirage_uapi::ioc::DRM_COMMAND_BASE + nr,
        core::mem::size_of::<T>() as u32,
    )
}

pub(crate) const fn drm_iowr<T>(nr: u32) -> u32 {
    mirage_uapi::ioc::iowr(
        mirage_uapi::ioc::DRM_MAGIC,
        mirage_uapi::ioc::DRM_COMMAND_BASE + nr,
        core::mem::size_of::<T>() as u32,
    )
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub(crate) struct DrmBoListEntry {
    pub bo_handle: u32,
    pub bo_priority: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub(crate) struct DrmCsChunk {
    pub chunk_id: u32,
    pub length_dw: u32,
    pub chunk_data: u64,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub(crate) struct DrmCsChunkDep {
    pub ip_type: u32,
    pub ip_instance: u32,
    pub ring: u32,
    pub ctx_id: u32,
    pub handle: u64,
}

impl RealEmulator {
    pub(crate) unsafe fn kfd_ioctl<T>(&self, nr: u32, arg: &mut T) -> AmdgpuResult<()> {
        unsafe { raw_ioctl(self.kfd.as_raw_fd(), nr, arg) }
    }

    pub(crate) unsafe fn drm_ioctl<T>(&self, nr: u32, arg: &mut T) -> AmdgpuResult<()> {
        let fd = self.primary_render_fd().map_err(from_io_error)?;
        unsafe { raw_ioctl(fd, nr, arg) }
    }
}

impl RealEmulator {
    pub(crate) fn kfd_get_version(&self) -> AmdgpuResult<AmdkfdIocGetVersionResponse> {
        let mut args = kfd::kfd_ioctl_get_version_args::default();
        // SAFETY: `args` has the exact layout the kernel expects.
        unsafe { self.kfd_ioctl(kfd_ior::<kfd::kfd_ioctl_get_version_args>(0x01), &mut args)? };
        Ok(AmdkfdIocGetVersionResponse {
            major_version: args.major_version,
            minor_version: args.minor_version,
        })
    }
}
