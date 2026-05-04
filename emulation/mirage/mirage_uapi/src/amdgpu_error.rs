//! Typed POSIX-style error enum for the amdgpu proxy / simulator.
//!
//! The numeric values follow the Linux UAPI (`<errno.h>`) and match the
//! values the AMD `amdgpu` and `kfd` kernel drivers return. Use
//! [`AmdgpuError::from_errno`] / [`AmdgpuError::errno`] to round-trip
//! between a raw errno integer and the typed variant.
//!
//! The type implements [`std::error::Error`] and [`std::fmt::Display`] so
//! it can be used directly as the `E` in `Result<T, AmdgpuError>`.

use serde::{Deserialize, Serialize};

/// Internal helper: declare [`AmdgpuError`] together with
/// [`AmdgpuError::from_errno`], [`AmdgpuError::errno`], and
/// [`AmdgpuError::name`] from a single variant/errno table, so adding a
/// new errno only touches one place. The short name string (e.g.
/// `"EINVAL"`) is derived from the `libc` identifier via `stringify!`.
macro_rules! amdgpu_errors {
    (
        $(
            $(#[$vmeta:meta])*
            $variant:ident = $errno:ident
        ),* $(,)?
    ) => {
        /// A typed POSIX-style error returned by an amdgpu-proxy operation.
        ///
        /// The numeric values follow the Linux UAPI (`<errno.h>`) and match
        /// the values the AMD `amdgpu` and `kfd` kernel drivers return.
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[non_exhaustive]
        pub enum AmdgpuError {
            /// No error (`0`). Provided so an [`AmdgpuError`] round-trips
            /// losslessly through [`AmdgpuError::from_errno`] /
            /// [`AmdgpuError::errno`]; APIs that distinguish success from
            /// failure should use `Result<_, AmdgpuError>` and never
            /// construct this.
            Success,

            $(
                $(#[$vmeta])*
                $variant,
            )*

            /// Catch-all for any errno not enumerated above. The wrapped
            /// value is the positive errno number as returned by the
            /// kernel.
            Other(i32),
        }

        impl AmdgpuError {
            /// Convert a raw (positive) errno value into a typed
            /// [`AmdgpuError`]. Unknown values become
            /// [`AmdgpuError::Other`].
            pub fn from_errno(errno: i32) -> Self {
                match errno {
                    0 => Self::Success,
                    $( n if n == libc::$errno => Self::$variant, )*
                    other => Self::Other(other),
                }
            }

            /// The raw (positive) errno integer corresponding to this
            /// variant.
            pub fn errno(&self) -> i32 {
                match self {
                    Self::Success => 0,
                    $( Self::$variant => libc::$errno, )*
                    Self::Other(n) => *n,
                }
            }

            /// A short, human-readable name for this error
            /// (e.g. `"EINVAL"`).
            pub fn name(&self) -> &'static str {
                match self {
                    Self::Success => "SUCCESS",
                    $( Self::$variant => stringify!($errno), )*
                    Self::Other(_) => "EUNKNOWN",
                }
            }
        }
    };
}

amdgpu_errors! {
    // -- Most common ioctl-style failures ----------------------------------
    /// `EPERM` — operation not permitted.
    Perm              = EPERM,
    /// `ENOENT` — no such file or directory / object.
    NoEntry           = ENOENT,
    /// `ESRCH` — no such process.
    NoProcess         = ESRCH,
    /// `EINTR` — interrupted system call.
    Interrupted       = EINTR,
    /// `EIO` — I/O error (often returned when a transport step fails).
    Io                = EIO,
    /// `ENXIO` — no such device or address.
    NoDeviceOrAddress = ENXIO,
    /// `E2BIG` — argument list too long.
    TooBig            = E2BIG,
    /// `EBADF` — bad file descriptor (e.g. unknown KFD/DRM fd).
    BadFd             = EBADF,
    /// `EAGAIN` / `EWOULDBLOCK` — resource temporarily unavailable.
    Again             = EAGAIN,
    /// `ENOMEM` — out of memory (driver heap, BO allocation, …).
    NoMemory          = ENOMEM,
    /// `EACCES` — permission denied.
    Access            = EACCES,
    /// `EFAULT` — bad address (user pointer rejected by the kernel).
    Fault             = EFAULT,
    /// `EBUSY` — device or resource busy.
    Busy              = EBUSY,
    /// `EEXIST` — object already exists.
    Exists            = EEXIST,
    /// `ENODEV` — no such device (renderD node missing, GPU absent).
    NoDevice          = ENODEV,
    /// `ENOTDIR` — not a directory.
    NotDir            = ENOTDIR,
    /// `EISDIR` — is a directory.
    IsDir             = EISDIR,
    /// `EINVAL` — invalid argument (by far the most common ioctl failure).
    Invalid           = EINVAL,
    /// `ENFILE` — too many open files in system.
    NoFileTable       = ENFILE,
    /// `EMFILE` — too many open files (per process).
    NoFileDescriptors = EMFILE,
    /// `ENOTTY` — inappropriate ioctl for device.
    NoTty             = ENOTTY,
    /// `EFBIG` — file/object too large.
    TooLarge          = EFBIG,
    /// `ENOSPC` — no space left on device.
    NoSpace           = ENOSPC,
    /// `EROFS` — read-only filesystem.
    ReadOnlyFs        = EROFS,
    /// `EPIPE` — broken pipe / peer closed.
    BrokenPipe        = EPIPE,
    /// `ERANGE` — result out of range.
    OutOfRange        = ERANGE,
    /// `ENAMETOOLONG` — name too long.
    NameTooLong       = ENAMETOOLONG,

    // -- Async / wait / queue specific -------------------------------------
    /// `ENOSYS` — function not implemented (simulator returns this for
    /// unsupported ops).
    NoSys             = ENOSYS,
    /// `ENOTEMPTY` — directory not empty.
    NotEmpty          = ENOTEMPTY,
    /// `ELOOP` — too many levels of symbolic links.
    Loop              = ELOOP,
    /// `EDEADLK` — resource deadlock would occur.
    Deadlock          = EDEADLK,
    /// `EOVERFLOW` — value too large to be stored in data type.
    Overflow          = EOVERFLOW,

    // -- Network / async-style ---------------------------------------------
    /// `ECONNRESET` — connection reset by peer.
    ConnectionReset   = ECONNRESET,
    /// `ETIMEDOUT` — connection / wait timed out.
    TimedOut          = ETIMEDOUT,
    /// `ECONNREFUSED` — connection refused (proxy server not running).
    ConnectionRefused = ECONNREFUSED,
    /// `EOPNOTSUPP` / `ENOTSUP` — operation not supported on socket /
    /// device. (`ENOTSUP == EOPNOTSUPP` on Linux, so they collapse to one
    /// variant.)
    OpNotSupported    = EOPNOTSUPP,

    // -- amdgpu / KFD specific (still standard errnos) ---------------------
    /// `EHWPOISON` — memory page has hardware error (HBM ECC, GPU fault).
    HardwareFault     = EHWPOISON,
    /// `EREMOTEIO` — remote I/O error.
    RemoteIo          = EREMOTEIO,
}

impl std::fmt::Display for AmdgpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name(), self.errno())
    }
}

impl std::error::Error for AmdgpuError {}

impl From<AmdgpuError> for i32 {
    fn from(err: AmdgpuError) -> i32 {
        err.errno()
    }
}

impl From<i32> for AmdgpuError {
    fn from(errno: i32) -> Self {
        Self::from_errno(errno)
    }
}

/// Convenience alias for operations that can fail with an [`AmdgpuError`].
pub type AmdgpuResult<T> = Result<T, AmdgpuError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_known() {
        for e in [
            AmdgpuError::Perm,
            AmdgpuError::Invalid,
            AmdgpuError::NoMemory,
            AmdgpuError::HardwareFault,
        ] {
            assert_eq!(AmdgpuError::from_errno(e.errno()), e);
        }
    }

    #[test]
    fn round_trip_success() {
        assert_eq!(AmdgpuError::from_errno(0), AmdgpuError::Success);
        assert_eq!(AmdgpuError::Success.errno(), 0);
    }

    #[test]
    fn unknown_errno_becomes_other() {
        assert_eq!(AmdgpuError::from_errno(9999), AmdgpuError::Other(9999));
        assert_eq!(AmdgpuError::Other(9999).errno(), 9999);
    }

    #[test]
    fn usable_as_result_error() {
        fn op() -> AmdgpuResult<u32> {
            Err(AmdgpuError::Invalid)
        }
        let err = op().unwrap_err();
        assert_eq!(err.to_string(), "EINVAL (22)");
    }
}
