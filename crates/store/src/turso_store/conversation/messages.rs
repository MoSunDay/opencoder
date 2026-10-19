use anyhow::{Context, Result};

use turso::{params, Connection};

use opencoder_core::{ContentBlock, Message, MessageUsage, Role};

use crate::types::{ImportReport, MessageChunkPage, MessageChunkRecord, MessageRow};

use opencoder_core::fleet::MessageCursor;
include!("messages/parts/part_1.rs");

include!("messages/parts/part_2.rs");
