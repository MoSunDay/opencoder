//! Schema 5: ordered parallel milestones with explicit reflection return paths.
//! The Brain evaluates business criteria after every terminal layer barrier.
//! Returning starts a fresh round while retaining every historical execution.
mod decision;
mod plan;
mod run;
pub use decision::*;
pub use plan::*;
pub use run::*;

pub const LAYERED_SCHEMA_VERSION: u32 = 5;
pub const LAYERED_MIGRATION: &str =
    "migration required: new layered brain runs require explicit schema_version: 5";

pub const LAYERED_MAX_NODES: usize = 256;
pub const LAYERED_MAX_LAYER_WIDTH: usize = 32;
pub const LAYERED_MAX_DEPTH: u32 = 3;
