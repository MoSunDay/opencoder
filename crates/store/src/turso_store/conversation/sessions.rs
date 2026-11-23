use anyhow::{Context, Result};

use turso::{params, params_from_iter, Connection, Value};

use crate::types::{SessionFilter, SessionListItem, SessionMeta, SessionPatch};
#[path = "sessions/activity.rs"]
pub(super) mod activity;
#[path = "sessions/listing.rs"]
mod listing;

include!("sessions/parts/part_1.rs");

include!("sessions/parts/part_2.rs");
