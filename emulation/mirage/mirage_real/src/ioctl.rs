use std::io;
use std::os::fd::AsRawFd;

use mirage_schema::amdgpu::AmdkfdIocGetVersionResponse;
use mirage_schema::amdgpu_error::{AmdgpuError, AmdgpuResult};
use mirage_uapi::ioctl::{self, IoctlCmd};
use mirage_uapi::kfd;

use crate::RealEmulator;

pub(crate) fn from_io_error(error: io::Error) -> AmdgpuError {
    error
        .raw_os_error()
        .map(AmdgpuError::from_errno)
        .unwrap_or(AmdgpuError::Io)
}

impl RealEmulator {
    pub(crate) fn kfd_ioctl<T>(&self, cmd: IoctlCmd<T>, arg: &mut T) -> AmdgpuResult<()> {
        ioctl::call(self.kfd.as_raw_fd(), cmd, arg).map_err(from_io_error)
    }

    pub(crate) fn drm_ioctl<T>(&self, cmd: IoctlCmd<T>, arg: &mut T) -> AmdgpuResult<()> {
        let fd = self.primary_render_fd().map_err(from_io_error)?;
        ioctl::call(fd, cmd, arg).map_err(from_io_error)
    }
}

impl RealEmulator {
    pub(crate) fn kfd_get_version(&self) -> AmdgpuResult<AmdkfdIocGetVersionResponse> {
        let mut args = kfd::kfd_ioctl_get_version_args::default();
        self.kfd_ioctl(
            ioctl::kfd_ior::<kfd::kfd_ioctl_get_version_args>(0x01),
            &mut args,
        )?;
        Ok(AmdkfdIocGetVersionResponse {
            major_version: args.major_version,
            minor_version: args.minor_version,
        })
    }
}
