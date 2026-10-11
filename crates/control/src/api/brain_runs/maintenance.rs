//! Reference checks for removing catalog entries without breaking consumers.
use crate::AppState;
use opencoder_core::fleet::ExecutionKind;
use serde_json::Value;

pub(crate) enum Removing<'a> {
    Plan(&'a str),
    Capability(&'a str),
}

impl Removing<'_> {
    fn matches_id(&self, value: &str) -> bool {
        match self {
            Self::Plan(id) => value.starts_with(&format!("plan-{id}@")),
            Self::Capability(id) => value == *id,
        }
    }

    fn referenced(&self, value: &Value) -> bool {
        match value {
            Value::Array(items) => items.iter().any(|item| self.referenced(item)),
            Value::Object(fields) => fields.iter().any(|(key, value)| {
                let matches = match key.as_str() {
                    "plan_id" => matches!(self, Self::Plan(id) if value.as_str() == Some(*id)),
                    "capability_id" | "id" => value.as_str().is_some_and(|id| self.matches_id(id)),
                    "capability_ids" => value.as_array().is_some_and(|ids| {
                        ids.iter()
                            .any(|id| id.as_str().is_some_and(|id| self.matches_id(id)))
                    }),
                    _ => false,
                };
                matches || self.referenced(value)
            }),
            _ => false,
        }
    }
}

/// Call while holding the exclusive brain-catalog registry lock. All saved
/// versions remain launchable, so references in older versions also block.
pub(crate) async fn references(
    state: &AppState,
    removing: Removing<'_>,
) -> anyhow::Result<Vec<String>> {
    let mut references = Vec::new();
    for definition in state.fleet.definitions("brain_plan").await? {
        let id = definition["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("saved plan id missing"))?;
        if matches!(removing, Removing::Plan(removed) if removed == id) {
            continue;
        }
        let mut before = None;
        loop {
            let versions = state.fleet.brain_plan_documents(id, before).await?;
            if versions.is_empty() {
                break;
            }
            before = versions.last().map(|version| version.version);
            for version in versions {
                if removing.referenced(&version.plan) {
                    references.push(format!("plan:{id}@{}", version.version));
                }
            }
        }
    }
    let mut cursor = None;
    loop {
        let page = state
            .fleet
            .indexes_page(None, Some(ExecutionKind::Brain), cursor.as_ref(), 100)
            .await?;
        for index in page.executions {
            if index.status.terminal() {
                continue;
            }
            let assignment =
                state.fleet.assignment(&index.id).await?.ok_or_else(|| {
                    anyhow::anyhow!("active Brain assignment missing: {}", index.id)
                })?;
            if removing.referenced(&assignment.request.input) {
                references.push(format!("run:{}", index.id));
            }
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    Ok(references)
}
