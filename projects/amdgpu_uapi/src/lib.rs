pub trait DecodeIoctl {
    /// Decode an ioctl from raw bytes.
    ///
    /// # Safety
    ///
    /// Caller must ensure `args` represents a valid argument buffer for
    /// the ioctl identified by `nr`; pointers inside the payload are
    /// dereferenced unchecked.
    unsafe fn decode_ioctl(nr: u32, args: &[u8]) -> Self;
}

pub trait EncodeIoctl {
    /// make an ioctl number and a byte slice of the arguments
    fn encode_ioctl(&self) -> (u32, &[u8]);
}
