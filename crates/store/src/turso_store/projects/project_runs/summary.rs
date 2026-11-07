use crate::ProjectRunText;

fn bounded_text(
    value: Option<String>,
    bytes: Option<i64>,
    field: String,
) -> Option<ProjectRunText> {
    bytes.map(|bytes| {
        if bytes > 64 * 1024 {
            ProjectRunText::Omitted {
                omitted: true,
                total_bytes: bytes.max(0) as u64,
                read_via: "detail_field",
                field,
            }
        } else {
            ProjectRunText::Text(value.unwrap_or_default())
        }
    })
}

pub(super) fn summary_text(
    value: Option<String>,
    bytes: Option<i64>,
    run_id: &str,
    field: &str,
) -> Option<ProjectRunText> {
    bounded_text(value, bytes, format!("project.run.{run_id}.{field}"))
}

pub(super) fn todo_text(
    value: Option<String>,
    bytes: Option<i64>,
    todo_id: &str,
    field: &str,
) -> Option<ProjectRunText> {
    bounded_text(value, bytes, format!("project.todo.{todo_id}.{field}"))
}
