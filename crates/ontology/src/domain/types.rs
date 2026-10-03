use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const VECTOR_DIMENSION: usize = 2048;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttributeKind {
    String,
    Integer,
    Float,
    Boolean,
    Datetime,
    Json,
    Vector,
    Text,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttributeRole {
    Custom,
    Source,
    Ext,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StorageMode {
    Sql,
    Markdown,
    NfsPath,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Environment {
    pub env_num: i64,
    pub env_key: String,
    pub name: String,
    pub description: String,
    pub initialization_status: String,
    pub revision: i64,
    pub is_deleted: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EntityType {
    pub id: Uuid,
    pub env_num: i64,
    pub type_key: String,
    pub name: String,
    pub description: String,
    pub is_system: bool,
    pub revision: i64,
    pub is_deleted: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AttributeDefinition {
    #[serde(
        serialize_with = "serialize_numeric_id",
        deserialize_with = "deserialize_numeric_id"
    )]
    pub id: i64,
    pub env_num: i64,
    pub entity_type_id: Uuid,
    pub attribute_key: String,
    pub name: String,
    pub description: String,
    pub kind: AttributeKind,
    pub attribute_role: AttributeRole,
    pub storage_mode: StorageMode,
    pub required: bool,
    pub revision: i64,
    pub is_deleted: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Entity {
    pub id: Uuid,
    pub env_num: i64,
    pub entity_type_id: Uuid,
    pub name: String,
    pub description: String,
    pub revision: i64,
    pub is_deleted: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RelationshipType {
    pub id: Uuid,
    pub env_num: i64,
    pub type_key: String,
    pub name: String,
    pub description: String,
    pub is_directory_membership: bool,
    pub is_system: bool,
    pub source_entity_type_id: Option<Uuid>,
    pub target_entity_type_ids: Vec<Uuid>,
    pub revision: i64,
    pub is_deleted: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EntityTypeAction {
    #[serde(
        serialize_with = "serialize_numeric_id",
        deserialize_with = "deserialize_numeric_id"
    )]
    pub id: i64,
    pub env_num: i64,
    pub entity_type_id: Uuid,
    pub operation_type: String,
    pub operation: String,
    pub description: String,
    pub revision: i64,
    pub is_deleted: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Relationship {
    pub id: Uuid,
    pub env_num: i64,
    pub relationship_type_id: Uuid,
    pub source_entity_id: Uuid,
    pub target_entity_id: Uuid,
    pub description: String,
    pub revision: i64,
    pub is_deleted: bool,
    pub is_pinned: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GraphAspect {
    pub id: Uuid,
    pub env_num: i64,
    pub aspect_key: String,
    pub name: String,
    pub description: String,
    pub entity_type_ids: Vec<Uuid>,
    pub relationship_type_ids: Vec<Uuid>,
    #[serde(flatten)]
    pub defaults: AspectDefaults,
    pub revision: i64,
    pub is_deleted: bool,
}

#[derive(Clone, Debug)]
pub struct AspectPatch {
    pub aspect_key: String,
    pub name: String,
    pub description: String,
    pub entity_type_ids: Vec<Uuid>,
    pub relationship_type_ids: Vec<Uuid>,
    pub defaults: AspectDefaults,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AspectDefaults {
    #[serde(default)]
    pub default_center_ids: Vec<Uuid>,
    pub default_upstream_depth: Option<u8>,
    pub default_downstream_depth: Option<u8>,
}

// Serde serialization callbacks receive references.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn serialize_numeric_id<S: serde::Serializer>(id: &i64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&id.to_string())
}

fn deserialize_numeric_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        Number(i64),
        String(String),
    }
    match Id::deserialize(deserializer)? {
        Id::Number(id) => Ok(id),
        Id::String(id) => id.parse().map_err(serde::de::Error::custom),
    }
}
