//! Schema version 4: the layered capability canvas.
//!
//! A v4 plan is a DAG of single-capability nodes. The scheduler derives layers
//! from the edges (Kahn), decides one layer at a time, dispatches every node of
//! that layer in parallel, and only advances after the whole layer is done.
//! Layers are never stored; they are recomputed from the plan everywhere.
mod decision;
mod plan;
mod run;
pub use decision::*;
pub use plan::*;
pub use run::*;

pub const LAYERED_SCHEMA_VERSION: u32 = 4;
pub const LAYERED_MIGRATION: &str =
    "migration required: new layered brain runs require explicit schema_version: 4";

pub const LAYERED_MAX_NODES: usize = 256;
pub const LAYERED_MAX_LAYER_WIDTH: usize = 32;
pub const LAYERED_MAX_DEPTH: u32 = 3;
