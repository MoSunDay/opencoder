//! Process-level layered canvas (schema_version 4): the admission gates, the
//! frozen `/layered` read surface and one complete canvas turn on a real
//! server + node + runc activation.
mod admission;
mod canvas;
#[allow(dead_code)]
mod fixtures;
#[path = "../support/mod.rs"]
mod support;
mod surface;
