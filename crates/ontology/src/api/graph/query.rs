use crate::error::AppError;
use uuid::Uuid;

#[derive(Default, Debug)]
pub(super) struct GraphQuery {
    pub centers: Vec<Uuid>,
    pub depth: Option<u8>,
    pub upstream_depth: Option<u8>,
    pub downstream_depth: Option<u8>,
    pub pinned_only: bool,
    pub expand_neighbors: bool,
    pub entity_type_ids: Vec<Uuid>,
    pub relationship_type_ids: Vec<Uuid>,
}

impl GraphQuery {
    pub fn parse(raw: &str) -> Result<Self, AppError> {
        let mut query = Self::default();
        for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
            let invalid = || AppError::invalid(format!("invalid graph parameter {key}"));
            match key.as_ref() {
                "center" => query.centers.push(value.parse().map_err(|_| invalid())?),
                "depth" => query.depth = Some(value.parse().map_err(|_| invalid())?),
                "upstream_depth" => {
                    query.upstream_depth = Some(value.parse().map_err(|_| invalid())?);
                }
                "downstream_depth" => {
                    query.downstream_depth = Some(value.parse().map_err(|_| invalid())?);
                }
                "pinned_only" => query.pinned_only = value.parse().map_err(|_| invalid())?,
                "expand_neighbors" => {
                    query.expand_neighbors = value.parse().map_err(|_| invalid())?;
                }
                "entity_type_id" => query
                    .entity_type_ids
                    .push(value.parse().map_err(|_| invalid())?),
                "relationship_type_id" => {
                    query
                        .relationship_type_ids
                        .push(value.parse().map_err(|_| invalid())?);
                }
                _ => return Err(invalid()),
            }
        }
        query.centers.sort_unstable();
        query.centers.dedup();
        Ok(query)
    }
}
