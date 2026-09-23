use anyhow::{ensure, Context, Result};
use serde_json::Value;

/// One JSON document; a presentation fence never changes the output contract.
pub fn parse_output(value: Value) -> Result<Value> {
    let Some(text) = value.as_str() else {
        return Ok(value);
    };
    ensure!(text.len() <= 256 * 1024, "PC output exceeds 256 KiB");
    let text = text.trim();
    // A unique, explicitly marked JSON block is the machine report. Surrounding
    // human-readable commentary is not part of that report. Never guess between
    // multiple blocks or extract unmarked braces from prose.
    let document = if text.contains("```") {
        let parts: Vec<_> = text.split("```").collect();
        ensure!(parts.len() == 3, "PC output requires one closed JSON fence");
        parts[1]
            .strip_prefix("json\n")
            .or_else(|| parts[1].strip_prefix("json\r\n"))
            .context("PC output fence must be marked json")?
            .trim()
    } else {
        text
    };
    serde_json::from_str(document).context("PC output must contain exactly one JSON document")
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .with_context(|| format!("{key} required"))
}
fn nonempty<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    value[key]
        .as_array()
        .filter(|v| !v.is_empty())
        .with_context(|| format!("nonempty {key} required"))
}

/// Structural gate, not a replacement for independently reading evidence.
pub fn validate_output(stage: &str, value: &Value) -> Result<()> {
    ensure!(
        value["schema"] == "pc-issue.stage/v1" && value["stage"] == stage,
        "PC issue output schema/stage mismatch"
    );
    text(value, "summary")?;
    let outcome = text(value, "outcome")?;
    let allowed: &[&str] = match stage {
        "impact" => &["mapped", "incomplete"],
        "reproduce" => &["reproduced", "not_reproduced", "blocked"],
        "repair" => &["changed", "not_needed", "blocked"],
        "verify" => &["verified", "failed", "not_needed", "blocked"],
        "conclude" => &["fixed", "diagnosed", "not_reproduced", "unresolved"],
        _ => anyhow::bail!("unknown PC issue stage"),
    };
    ensure!(allowed.contains(&outcome), "invalid PC issue outcome");
    let evidence = value["evidence"]
        .as_array()
        .context("evidence array required")?;
    for item in evidence {
        text(item, "path")?;
        let sha = text(item, "sha256")?;
        ensure!(
            sha.len() == 64 && sha.bytes().all(|c| c.is_ascii_hexdigit()),
            "invalid evidence hash"
        );
        ensure!(
            item["bytes"].as_u64().is_some_and(|n| n > 0),
            "evidence bytes required"
        );
    }
    if matches!(
        outcome,
        "mapped" | "reproduced" | "changed" | "verified" | "fixed" | "diagnosed"
    ) {
        ensure!(!evidence.is_empty(), "successful claim requires evidence");
    }
    match stage {
        "impact" => {
            ensure!(
                value["source_baselines"].is_array(),
                "source_baselines array required"
            );
            if outcome == "mapped" {
                nonempty(value, "source_baselines")?;
            }
            ensure!(
                value["hypotheses"].is_array() && value["gaps"].is_array(),
                "hypotheses and gaps required"
            );
        }
        "reproduce" if outcome != "blocked" => {
            text(value, "device_execution_id")?;
            text(value, "app_version")?;
            ensure!(
                value["restoration_verified"] == true,
                "device restoration not verified"
            );
            nonempty(value, "observations")?;
            ensure!(value["capture"].is_object(), "capture coverage required");
        }
        "repair" if outcome == "changed" => {
            nonempty(value, "changes")?;
            text(value, "worktree")?;
            ensure!(
                matches!(value["purpose"].as_str(), Some("diagnostic" | "fix")),
                "change purpose required"
            );
        }
        "verify" if outcome == "verified" => {
            text(value, "device_execution_id")?;
            text(value, "build_execution_id")?;
            nonempty(value, "artifacts")?;
            nonempty(value, "assertions")?;
            ensure!(
                value["assertions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|a| a["passed"] == true),
                "original assertions did not all pass"
            );
            ensure!(
                value["restoration_verified"] == true && value["download_verified"] == true,
                "restoration and artifact download verification required"
            );
        }
        "conclude" if outcome == "fixed" => {
            ensure!(
                value["baseline_reproduced"] == true
                    && value["fix_verified"] == true
                    && value["restoration_verified"] == true,
                "fixed requires baseline reproduction, verified fix and restoration"
            );
            nonempty(value, "verification_execution_ids")?;
        }
        _ => {}
    }
    if matches!(
        outcome,
        "blocked" | "incomplete" | "not_needed" | "unresolved"
    ) {
        text(value, "reason")?;
    }
    Ok(())
}

/// Cross-stage assertions use host-loaded ancestor results, never model-written history.
pub fn validate_transition(stage: &str, history: &Value, output: &Value) -> Result<()> {
    if stage == "verify" && output["outcome"] == "verified" {
        ensure!(
            history["repair"]["outcome"] == "changed",
            "verification requires a recorded candidate change"
        );
        ensure!(
            output["purpose"] == history["repair"]["purpose"],
            "candidate purpose changed"
        );
        ensure!(
            output["rounds"]
                .as_u64()
                .is_some_and(|n| (1..=2).contains(&n)),
            "verification round budget exceeded"
        );
    }
    if stage == "conclude" && output["outcome"] == "fixed" {
        ensure!(
            history["reproduce"]["outcome"] == "reproduced"
                && history["verify"]["outcome"] == "verified"
                && history["verify"]["purpose"] == "fix",
            "fixed conclusion is inconsistent with authoritative ancestor results"
        );
        let ids = output["verification_execution_ids"]
            .as_array()
            .context("verification executions required")?;
        ensure!(
            ids.contains(&history["verify"]["device_execution_id"])
                && ids.contains(&history["verify"]["build_execution_id"]),
            "conclusion omitted verified executions"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn cannot_claim_fixed_without_evidence_and_baseline() {
        let mut v = json!({"schema":"pc-issue.stage/v1","stage":"conclude","summary":"fixed","outcome":"fixed","evidence":[]});
        assert!(validate_output("conclude", &v).is_err());
        v["evidence"] = json!([{"path":"receipt.json","sha256":"a".repeat(64),"bytes":1}]);
        assert!(validate_output("conclude", &v).is_err());
        v["baseline_reproduced"] = json!(true);
        v["fix_verified"] = json!(true);
        v["restoration_verified"] = json!(true);
        v["verification_execution_ids"] = json!(["dag-retest"]);
        validate_output("conclude", &v).unwrap();
    }
    #[test]
    fn skipped_build_is_explicit_and_not_verified() {
        let v = json!({"schema":"pc-issue.stage/v1","stage":"verify","summary":"no change","outcome":"not_needed","reason":"baseline did not reproduce","evidence":[]});
        validate_output("verify", &v).unwrap();
        let mut bad = v;
        bad["outcome"] = json!("verified");
        assert!(validate_output("verify", &bad).is_err());
        assert!(validate_output("repair", &bad).is_err());
    }
    #[test]
    fn fixed_must_match_host_history_and_diagnostic_is_not_a_fix() {
        let mut history = json!({"reproduce":{"outcome":"reproduced"},"verify":{
            "outcome":"verified","purpose":"diagnostic","device_execution_id":"dag-v",
            "build_execution_id":"team-v"}});
        let mut result = json!({"outcome":"fixed","verification_execution_ids":["dag-v","team-v"]});
        assert!(validate_transition("conclude", &history, &result).is_err());
        history["verify"]["purpose"] = json!("fix");
        validate_transition("conclude", &history, &result).unwrap();
        result["verification_execution_ids"] = json!(["dag-other", "team-v"]);
        assert!(validate_transition("conclude", &history, &result).is_err());
    }
    #[test]
    fn unique_explicit_fence_preserves_report_and_rejects_ambiguity() {
        let raw = r#"{"schema":"pc-issue.stage/v1","stage":"impact"}"#;
        let value = parse_output(json!(raw)).unwrap();
        for wrapped in [
            format!("```json\n{raw}\n```"),
            format!("```json\r\n{raw}\r\n```\n"),
            format!("Report:\n```json\n{raw}\n```\nCompleted analysis."),
        ] {
            assert_eq!(parse_output(json!(wrapped)).unwrap(), value);
        }
        for bad in [
            format!("Result: {raw}"),
            format!("```json\n{raw}\n```\n```json\n{raw}\n```"),
            format!("```text\n{raw}\n```"),
            format!("```json\n{raw}"),
            format!("```json\n{raw} {raw}\n```"),
            format!("{raw} {raw}"),
        ] {
            assert!(parse_output(json!(bad)).is_err());
        }
    }
}
