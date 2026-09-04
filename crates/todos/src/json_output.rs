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
    let body = &trimmed[start + marker_len..];
    // The closing fence is the LAST bare ``` line (a raw newline followed by
    // ``` and nothing but whitespace to the end). Scanning for the first ```
    // truncates a JSON document that itself embeds backticks (e.g. a fenced
    // code snippet inside a result string) — inside JSON strings raw
    // newlines are escaped, so an embedded fence can never sit at a bare
    // line start and only the real closing fence matches.
    let mut close = None;
    let mut from = 0;
    while let Some(rel) = body[from..].find("\n```") {
        let at = from + rel;
        if body[at + 4..].trim().is_empty() {
            close = Some(at);
        }
        from = at + 4;
    }
    let close = close.ok_or_else(|| anyhow::anyhow!("unterminated JSON fence"))?;
    let inner = body[..close].trim();
    let value: T = serde_json::from_str(inner).map_err(|e| {
        if inner.contains("```") {
            // Unparseable AND still carrying fence marks: more than one
            // top-level fence (an embedded backtick run inside a valid
            // document parses above and never reaches this branch).
            anyhow::anyhow!("multiple JSON fences are not a structured response")
        } else {
            anyhow::anyhow!(e)
        }
    })?;
    Ok(value)
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

    /// T-12: a JSON string value may itself contain ``` (a fenced snippet in
    /// a result field). The first ``` must not truncate the document.
    #[test]
    fn accepts_fenced_json_embedding_backticks() {
        let raw = "```json\n{\"summary\":\"see\\n```rust\\nfn a(){}\\n```\",\"ok\":true}\n```";
        let value: serde_json::Value = parse(raw).unwrap();
        assert_eq!(value["ok"], true);
        assert!(value["summary"].as_str().unwrap().contains("fn a(){}"));
    }
}
