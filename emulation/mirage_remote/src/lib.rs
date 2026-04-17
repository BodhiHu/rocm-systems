//! Unix-socket remote transport for the Mirage [`Emulator`] trait.
//!
//! * [`RemoteEmulator`] is a client-side [`Emulator`] implementation that
//!   forwards every ioctl to a daemon.
//! * [`EmulatorServer`] is the matching daemon-side server.
//! * [`protocol`] defines the BARE-encoded wire format.
//!
//! [`Emulator`]: mirage_schema::emulator::Emulator

pub mod client;
pub mod protocol;
pub mod server;
pub mod transport;

pub use client::RemoteEmulator;
pub use protocol::{MAX_FRAME_LEN, WireRequest, WireResponse, WireResult};
pub use server::EmulatorServer;
pub use transport::{WireError, read_frame, write_frame};

#[cfg(test)]
mod tests;
