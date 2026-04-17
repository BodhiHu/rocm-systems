//! Idiomatic Rust types for the Mirage schema.


pub mod common;
pub mod config;
pub mod container;
pub mod ctl;
pub mod daemon;
pub mod paths;
pub mod simulator;
pub mod simulator_service;
pub mod socket;

mod syscall_macro;

pub mod emulator;
pub mod syscalls;
pub mod topology;


pub use mirage_macros::ctl_dsl;
pub use mirage_uapi::amdgpu;
pub use mirage_uapi::amdgpu_error;
