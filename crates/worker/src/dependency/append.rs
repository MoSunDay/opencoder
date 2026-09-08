//! Render a validated analysis result as a bounded how.md fragment and
//! persist it as a new prompt-pool version.
//!
//! Reuses the DAG runtime's append path so the fragment lands in the
//! executing persona's shared `how.md` exactly like a workflow-declared
//! `how_append` (versions only grow; every agent referencing the pool sees
//! it). Only `analyze` appends — `status` never writes, so resuming a poll
//! does not create duplicate versions.

use super::model::AnalysisResult;
use opencoder_dag::MAX_HOW_APPEND_BYTES;

/// Map the wire verdict to the Chinese label used in the analysis contract.
fn verdict_label(depend_type: &str) -> &'static str {
    match depend_type {
        "strong" => "强依赖",
        "weak" => "弱依赖",
        "strong_to_weak" => "强转弱",
        _ => "未知",
    }
}

fn pair_str(pair: &serde_json::Value, key: &str) -> String {
    pair.get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Build the how.md fragment. Bounded to [`MAX_HOW_APPEND_BYTES`]; the
/// summary is truncated head-first (its leading conclusion is the part that
/// matters) when the pair header is large.
pub fn format_delta(result: &AnalysisResult) -> String {
    let header = format!(
        "## 强弱依赖分析 {}\n\n- 调用方: `{}.{}` ({})\n- 被调方: `{}.{}`\n- 依赖类型: **{}** (`{}`)\n- 结论: ",
        result.task_id,
        pair_str(&result.pair, "source_psm"),
        pair_str(&result.pair, "source_method"),
        pair_str(&result.pair, "region"),
        pair_str(&result.pair, "target_psm"),
        pair_str(&result.pair, "target_method"),
        verdict_label(&result.depend_type),
        result.depend_type,
    );
    let budget = MAX_HOW_APPEND_BYTES.saturating_sub(header.len() + 2);
    let summary = truncate_bytes(&result.summary, budget);
    format!("{header}{summary}")
}

/// Truncate to at most `max` bytes on a UTF-8 char boundary, marking a cut.
fn truncate_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].trim_end()
}

/// Append the result to the persona's prompt-pool how.md, returning the new
/// pool version.
pub fn append_result(agent: &str, result: &AnalysisResult) -> Result<u32, String> {
    let delta = format_delta(result);
    opencoder_dag_runtime::exec::how_append::append_to_how_md(agent, &delta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn result(depend_type: &str, summary: &str) -> AnalysisResult {
        AnalysisResult {
            task_id: "dep-1".into(),
            status: "success".into(),
            depend_type: depend_type.into(),
            summary: summary.into(),
            pair: json!({
                "region": "cn",
                "source_psm": "a.b",
                "source_method": "Entry",
                "target_psm": "c.d",
                "target_method": "Callee",
            }),
        }
    }

    #[test]
    fn delta_contains_verdict_and_pair() {
        let delta = format_delta(&result("strong", "同步调用，失败阻断主流程"));
        assert!(delta.contains("强依赖"));
        assert!(delta.contains("`a.b.Entry`"));
        assert!(delta.contains("`c.d.Callee`"));
        assert!(delta.contains("dep-1"));
        assert!(delta.contains("同步调用"));
    }

    #[test]
    fn delta_respects_size_bound() {
        let huge = "结论".repeat(MAX_HOW_APPEND_BYTES);
        let delta = format_delta(&result("weak", &huge));
        assert!(delta.len() <= MAX_HOW_APPEND_BYTES);
        assert!(delta.contains("弱依赖"));
    }

    #[test]
    fn truncate_keeps_char_boundary() {
        let s = "中文结论abc";
        let cut = truncate_bytes(s, 4);
        assert!(s.starts_with(cut));
        assert!(cut.len() <= 4);
    }

    /// The agents-dir override is process-global; serialize tests that flip it.
    static OVERRIDE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn append_result_bumps_prompt_pool_version() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = OVERRIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        opencoder_core::agent::set_agents_dir_override(Some(tmp.path().to_path_buf()));

        // Seed the persona's prompts pool v1 with a seed how.md.
        use opencoder_agents::write::{save_resource_version, VersionFile};
        save_resource_version(
            "prompts",
            "dependency-analysis",
            &[VersionFile {
                rel_path: "how.md".into(),
                bytes: "seed knowledge".as_bytes().to_vec(),
            }],
        )
        .unwrap();

        let version = append_result("dependency-analysis", &result("strong", "同步阻断")).unwrap();
        assert_eq!(version, 2);
        let grown =
            std::fs::read_to_string(tmp.path().join("prompts/dependency-analysis/v2/how.md"))
                .unwrap();
        assert!(grown.contains("seed knowledge"));
        assert!(grown.contains("强依赖"));
        assert!(grown.contains("同步阻断"));

        opencoder_core::agent::set_agents_dir_override(None);
    }
}
