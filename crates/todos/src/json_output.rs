use anyhow::Result;
use serde::de::DeserializeOwned;

/// Parse a structured model response while tolerating one complete Markdown fence.
/// Explanatory text outside the JSON remains invalid.
pub fn parse<T: DeserializeOwned>(raw: &str) -> Result<T> {
    let trimmed = raw.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }
    let (start, marker_len) = if let Some(index) = trimmed.find("```json") {
        (index, "```json".len())
    } else if let Some(index) = trimmed.find("```") {
        (index, "```".len())
    } else {
        return Ok(serde_json::from_str(trimmed)?);
    };
    let body_start = start + marker_len;
    let relative_end = trimmed[body_start..]
        .find("```")
        .ok_or_else(|| anyhow::anyhow!("unterminated JSON fence"))?;
    let body_end = body_start + relative_end;
    if trimmed[body_end + 3..].contains("```") {
        anyhow::bail!("multiple JSON fences are not a structured response");
    }
    Ok(serde_json::from_str(trimmed[body_start..body_end].trim())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_raw_or_one_complete_fence() {
        for raw in [
            "{\"ok\":true}",
            "```json\n{\"ok\":true}\n```",
            "```\n{\"ok\":true}\n```",
        ] {
            let value: serde_json::Value = parse(raw).unwrap();
            assert_eq!(value["ok"], true);
        }
    }

    #[test]
    fn rejects_explanation_around_json() {
        assert!(parse::<serde_json::Value>("result: {\"ok\":true}").is_err());
    }

    #[test]
    fn accepts_one_fenced_object_after_a_short_explanation() {
        let value: serde_json::Value =
            parse("blocked by runtime\n```json\n{\"status\":\"blocked\"}\n```").unwrap();
        assert_eq!(value["status"], "blocked");
    }

    #[test]
    fn rejects_multiple_fences() {
        assert!(parse::<serde_json::Value>("```json\n{}\n``` then ```json\n{}\n```").is_err());
    }
}
