use crate::api_project_util::error_400;
use axum::response::Response;
use opencoder_store::ProjectExecutorKind;
use serde::Deserialize;
/// Force deserialization of the INNER `Option<T>` so JSON `null` produces
/// `Some(None)` (clear) instead of collapsing to the outer `None` (absent).
pub fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<T>::deserialize(de)?))
}

/// Flatten a doubled spec field into the plain value that lands in the
/// record: absent/null/blank ⇒ None, else the trimmed JSON text. Blank
/// normalizes to a clear so `{"executor_spec": ""}` cannot store junk.
pub fn flatten_spec(spec: Option<Option<String>>) -> Option<String> {
    spec.flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Trimmed executor ref; empty ⇒ None (agent falls back to `act`, team/dag
/// resolve lazily from their inline spec at execute time).
pub fn normalize_ref(raw: Option<&str>) -> Option<String> {
    let trimmed = raw.unwrap_or("").trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[allow(clippy::result_large_err)]
pub fn validate_capability(raw: Option<&str>) -> Result<Option<String>, Response> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some(value)
            if matches!(
                value,
                "operator" | "agent" | "team" | "dag" | "todos" | "brain"
            ) =>
        {
            Ok(Some(value.to_owned()))
        }
        Some(_) => Err(error_400("unsupported TODO capability")),
    }
}

/// Parse `executor_kind` (absent ⇒ default agent; unknown string → 400
/// `unknown executor_kind: {x}`) and validate the non-blank inline spec
/// against it via the store-side pure `validate_spec` (the canonical
/// validator shared with the control plane; it also owns the agent-with-
/// spec rejection, "agent executor takes no spec").
// `Response` (axum) is inherently large; boxing would ripple through every
// handler call site for no gain.
#[allow(clippy::result_large_err)]
pub fn validate_executor(
    kind: Option<&str>,
    spec: Option<&str>,
) -> Result<ProjectExecutorKind, Response> {
    let kind = match kind {
        None => ProjectExecutorKind::default(),
        Some(raw) => ProjectExecutorKind::parse(raw)
            .ok_or_else(|| error_400(format!("unknown executor_kind: {raw}")))?,
    };
    if let Some(spec) = spec.map(str::trim).filter(|s| !s.is_empty()) {
        if let Err(e) = opencoder_store::project_executor_spec::validate_spec(kind, spec) {
            return Err(error_400(format!("executor_spec: {e:#}")));
        }
    }
    Ok(kind)
}
