use crate::database::text_revision::TextContent;
use crate::domain::{AspectDefaults, AttributeKind, AttributeRole, StorageMode};
use serde_json::Value;
use uuid::Uuid;

pub(crate) struct EntityUpdate<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub deleted: bool,
    pub expected: i64,
}

pub(crate) struct AspectCreate<'a> {
    pub key: &'a str,
    pub title: &'a str,
    pub description: &'a str,
    pub types: Vec<Uuid>,
    pub relations: Vec<Uuid>,
    pub defaults: AspectDefaults,
}

pub(crate) struct AspectUpdate<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub types: Vec<Uuid>,
    pub relations: Vec<Uuid>,
    pub defaults: AspectDefaults,
    pub deleted: bool,
    pub expected: i64,
}

pub(crate) struct AttributeCreate<'a> {
    pub key: &'a str,
    pub title: &'a str,
    pub description: &'a str,
    pub value_kind: AttributeKind,
    pub role: AttributeRole,
    pub storage: StorageMode,
    pub required: bool,
}

pub(crate) struct AttributeUpdate<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub required: bool,
    pub role: Option<AttributeRole>,
    pub storage: Option<StorageMode>,
    pub deleted: bool,
    pub expected: i64,
}

pub(crate) struct DefinitionUpdate<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub deleted: bool,
    pub expected: i64,
}

pub(crate) struct RelationshipTypeCreate<'a> {
    pub key: &'a str,
    pub title: &'a str,
    pub description: &'a str,
    pub source: Option<Uuid>,
    pub targets: &'a [Uuid],
}

pub(crate) struct RelationshipTypeUpdate<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub source: Option<Uuid>,
    pub targets: &'a [Uuid],
    pub deleted: bool,
    pub expected: i64,
}

pub(crate) struct RelationshipUpdate<'a> {
    pub description: &'a str,
    pub deleted: bool,
    pub pinned: Option<bool>,
    pub expected: i64,
}

pub(crate) struct TextWrite<'a> {
    pub env: i64,
    pub env_key: &'a str,
    pub entity: Uuid,
    pub attribute: i64,
    pub input: TextContent<'a>,
    pub required: bool,
    pub expected: i64,
}

pub(crate) struct StructuredWrite<'a> {
    pub kind: AttributeKind,
    pub value: &'a Value,
    pub deleted: bool,
    pub expected: i64,
}

pub(crate) struct VectorWrite<'a> {
    pub requested: Option<Uuid>,
    pub values: &'a [f32],
    pub deleted: bool,
    pub expected: i64,
}
