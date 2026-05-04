use std::io;
use std::marker::PhantomData;
use std::mem::{size_of, size_of_val};
use std::os::fd::RawFd;
use std::slice;

use bytes::{BufMut, BytesMut};

use crate::ioc;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct IoctlCmd<T> {
    nr: u32,
    marker: PhantomData<fn(&mut T)>,
}

impl<T> IoctlCmd<T> {
    pub const fn new(nr: u32) -> Self {
        Self {
            nr,
            marker: PhantomData,
        }
    }

    pub const fn nr(self) -> u32 {
        self.nr
    }
}

pub const fn kfd_ior<T>(nr: u32) -> IoctlCmd<T> {
    IoctlCmd::new(ioc::ior(ioc::KFD_MAGIC, nr, size_of::<T>() as u32))
}

pub const fn kfd_iow<T>(nr: u32) -> IoctlCmd<T> {
    IoctlCmd::new(ioc::iow(ioc::KFD_MAGIC, nr, size_of::<T>() as u32))
}

pub const fn kfd_iowr<T>(nr: u32) -> IoctlCmd<T> {
    IoctlCmd::new(ioc::iowr(ioc::KFD_MAGIC, nr, size_of::<T>() as u32))
}

pub const fn drm_iow<T>(nr: u32) -> IoctlCmd<T> {
    IoctlCmd::new(ioc::iow(
        ioc::DRM_MAGIC,
        ioc::DRM_COMMAND_BASE + nr,
        size_of::<T>() as u32,
    ))
}

pub const fn drm_iowr<T>(nr: u32) -> IoctlCmd<T> {
    IoctlCmd::new(ioc::iowr(
        ioc::DRM_MAGIC,
        ioc::DRM_COMMAND_BASE + nr,
        size_of::<T>() as u32,
    ))
}

pub fn call<T>(fd: RawFd, cmd: IoctlCmd<T>, arg: &mut T) -> io::Result<()> {
    let rc = unsafe { libc::ioctl(fd, cmd.nr() as _, arg as *mut T) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn maybe_ptr<T>(slice: &[T]) -> u64 {
    if slice.is_empty() {
        0
    } else {
        slice.as_ptr() as usize as u64
    }
}

pub fn maybe_mut_ptr<T>(slice: &mut [T]) -> u64 {
    if slice.is_empty() {
        0
    } else {
        slice.as_mut_ptr() as usize as u64
    }
}

pub fn bytes_of_value<T>(value: &T) -> Vec<u8> {
    let raw = unsafe { slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) };
    let mut buffer = BytesMut::with_capacity(raw.len());
    buffer.put_slice(raw);
    buffer.to_vec()
}

pub fn bytes_of_slice<T>(values: &[T]) -> Vec<u8> {
    let raw = unsafe { slice::from_raw_parts(values.as_ptr().cast::<u8>(), size_of_val(values)) };
    let mut buffer = BytesMut::with_capacity(raw.len());
    buffer.put_slice(raw);
    buffer.to_vec()
}
