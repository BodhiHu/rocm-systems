//! Daemon configuration types.
//!
//! This module corresponds to the `config.fbs` FlatBuffer schema
//! (`namespace mirage.config`).  It defines the top-level configuration
//! structure the daemon reads at startup.

use serde::{Deserialize, Serialize};

use crate::common::ProfileDef;

/// Top-level daemon configuration.
///
/// Loaded from a `.mcfg` file (FlatBuffer file identifier `"MIRC"`) or,
/// in the Rust world, from JSON / TOML / YAML via serde.
///
/// Contains the list of [`ProfileDef`]s that the daemon makes available
/// to clients and the dashboard.
///
/// # Examples
///
/// ```
/// # use mirage_schema::config::DaemonDef;
/// # use mirage_schema::common::{ProfileDef, SimulatorMode};
/// let cfg = DaemonDef {
///     profiles: vec![ProfileDef {
///         name: "mi300x-functional".into(),
///         simulator: "rocjitsu".into(),
///         mode: SimulatorMode::Functional,
///         gpu: "MI300X".into(),
///         num_gpus: 1,
///         num_nodes: 1,
///     }],
/// };
/// assert_eq!(cfg.profiles.len(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonDef {
    /// The profiles this daemon instance exposes.
    pub profiles: Vec<ProfileDef>,
}
