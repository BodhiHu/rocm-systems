//! Synchronous, length-prefixed BARE framing over a [`UnixStream`].
//!
//! The RemoteEmulator runs on the ioctl hot path — inside real
//! applications intercepted by `LD_PRELOAD` — so we avoid any async
//! runtime and use plain blocking I/O. The daemon side uses the same
//! code for symmetry.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::protocol::MAX_FRAME_LEN;

/// Errors produced by the transport layer.
#[derive(Debug)]
pub enum WireError {
    Io(io::Error),
    Encode(serde_bare::error::Error),
    Decode(serde_bare::error::Error),
    /// Peer announced a frame larger than [`MAX_FRAME_LEN`].
    FrameTooLarge(usize),
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "wire io error: {e}"),
            Self::Encode(e) => write!(f, "wire encode error: {e}"),
            Self::Decode(e) => write!(f, "wire decode error: {e}"),
            Self::FrameTooLarge(n) => {
                write!(f, "wire frame of {n} bytes exceeds maximum {MAX_FRAME_LEN}")
            }
        }
    }
}

impl std::error::Error for WireError {}

impl From<io::Error> for WireError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Serialize `value` with serde_bare, prefix with a big-endian u32 length
/// and flush to `stream`.
pub fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<(), WireError> {
    let payload = serde_bare::to_vec(value).map_err(WireError::Encode)?;
    if payload.len() > MAX_FRAME_LEN {
        return Err(WireError::FrameTooLarge(payload.len()));
    }
    let len = payload.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&payload)?;
    stream.flush()?;
    Ok(())
}

/// Read a length-prefixed frame and decode it with serde_bare.
pub fn read_frame<T: DeserializeOwned>(stream: &mut UnixStream) -> Result<T, WireError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_LEN {
        return Err(WireError::FrameTooLarge(len));
    }
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload)?;
    serde_bare::from_slice(&payload).map_err(WireError::Decode)
}
