use std::io;
use std::os::fd::AsRawFd;

use mirage_schema::amdgpu::AmdkfdIocGetVersionResponse;
use mirage_schema::amdgpu_error::{AmdgpuError, AmdgpuResult};

use crate::RealEmulator;

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

#[repr(C)]
struct KfdIocGetVersionArgs {
    major: u32,
    minor: u32,
}

impl RealEmulator {
    pub(crate) fn kfd_get_version(&self) -> AmdgpuResult<AmdkfdIocGetVersionResponse> {
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
