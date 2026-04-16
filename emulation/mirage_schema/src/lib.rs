//! Idiomatic Rust types for the Mirage schema.
//!
//! This crate is a **pure-Rust, serde-native** translation of the Mirage
//! FlatBuffer schemas (`schema/*.fbs`).  Every FlatBuffer `table` becomes a
//! Rust `struct`, every `enum` becomes a Rust `enum`, and all types derive
//! [`serde::Serialize`] / [`serde::Deserialize`] so they can be used with
//! any serde-compatible format (JSON, MessagePack, TOML, …).
//!
//! # Module layout
//!
//! The module structure mirrors the `.fbs` file structure:
//!
//! | Module                 | Schema file              | FlatBuffer namespace     |
//! |------------------------|--------------------------|--------------------------|
//! | [`common`]             | `common.fbs`             | `mirage.fb`              |
//! | [`config`]             | `config.fbs`             | `mirage.config`          |
//! | [`container`]          | `container.fbs`          | `mirage.container`       |
//! | [`simulator`]          | `simulator.fbs`          | `mirage.simulator`       |
//! | [`simulator_service`]  | `simulator_service.fbs`  | `mirage.simulator` (RPC) |
//! | [`socket`]             | `socket.fbs`             | `mirage.socket`          |
//!
//! # Quick start
//!
//! ```rust
//! use mirage_schema::common::{ProfileDef, SimulatorMode};
//!
//! let profile = ProfileDef {
//!     name: "mi300x-functional".into(),
//!     simulator: "rocjitsu".into(),
//!     mode: SimulatorMode::Functional,
//!     gpu: "MI300X".into(),
//!     num_gpus: 1,
//!     num_nodes: 1,
//! };
//!
//! let json = serde_json::to_string_pretty(&profile).unwrap();
//! println!("{json}");
//! ```

pub mod common;
pub mod config;
pub mod container;
pub mod simulator;
pub mod simulator_service;
pub mod socket;

