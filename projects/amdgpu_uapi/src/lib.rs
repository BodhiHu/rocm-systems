pub trait DecodeIoctl {
    // it's not safe to decode an ioctl becuse the data could point anywhere
    unsafe fn decode_ioctl(nr: u32, args: &[u8]) -> Self;   
}

pub trait EncodeIoctl {
    /// make an ioctl number and a byte slice of the arguments 
    fn encode_ioctl(&self) -> (u32, &[u8]);
}
