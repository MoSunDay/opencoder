//! Conversation domain: sessions, inputs, messages, events and subagent
//! tasks over the rdb KV engine. Free functions take `&rdb::store::Store`
//! (already behind the engine's write lease) so trait methods stay thin.

pub mod events;
pub mod inputs;
pub mod list;
pub mod messages;
pub mod sessions;
pub mod subagent_tasks;
