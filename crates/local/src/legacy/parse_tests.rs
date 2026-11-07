//! Strict-decoding unit tests for `parse.rs`: unknown enum strings,
//! malformed JSON columns, floating timestamps and non-0/1 flags are errors.

use crate::legacy::parse;
use crate::legacy::rows::{LegacyEvent, LegacyInput};

#[test]
fn parse_message_is_strict_about_every_cell() {
    let usage =
        serde_json::json!({"input_tokens": 1, "output_tokens": 2, "total_tokens": 3}).to_string();
    let blocks = r#"[{"kind":"text","text":"hi"}]"#.to_string();
    let row = |role: &str, blocks: &str, created: serde_json::Value| {
        vec![
            serde_json::json!("s1"),
            serde_json::json!("m1"),
            serde_json::json!(role),
            serde_json::Value::Null,
            serde_json::Value::Null,
            serde_json::json!(blocks),
            serde_json::Value::Null,
            serde_json::json!(usage),
            created,
            serde_json::json!(0),
            serde_json::Value::Null,
        ]
    };
    let ok = parse::parse_message(&row("user", &blocks, serde_json::json!(10))).unwrap();
    assert_eq!(ok.message.text(), "hi");
    assert_eq!(ok.message.created_at, 10);

    assert!(
        parse::parse_message(&row("wizard", &blocks, serde_json::json!(10)))
            .unwrap_err()
            .to_string()
            .contains("unknown legacy message role")
    );
    assert!(
        parse::parse_message(&row("user", "[{", serde_json::json!(10)))
            .unwrap_err()
            .to_string()
            .contains("blocks_json")
    );
    let err = parse::parse_message(&row("user", &blocks, serde_json::json!(10.5))).unwrap_err();
    assert!(err.to_string().contains("INTEGER"), "float ts: {err}");
}

#[test]
fn parse_flags_reject_values_outside_zero_one() {
    let base = |synthetic: serde_json::Value| {
        vec![
            serde_json::json!("s1"),
            serde_json::json!("m1"),
            serde_json::json!("user"),
            serde_json::Value::Null,
            serde_json::Value::Null,
            serde_json::json!(r#"[{"kind":"text","text":"hi"}]"#),
            serde_json::Value::Null,
            serde_json::json!(r#"{"input_tokens":0,"output_tokens":0,"total_tokens":0}"#),
            serde_json::json!(1),
            synthetic,
            serde_json::Value::Null,
        ]
    };
    assert!(parse::parse_message(&base(serde_json::json!(1))).is_ok());
    assert!(parse::parse_message(&base(serde_json::json!(2))).is_err());
    assert!(parse::parse_message(&base(serde_json::Value::Null)).is_err());
}

#[test]
fn parse_enums_reject_unknown_strings() {
    assert!(parse::parse_delivery("steer").is_ok());
    assert!(parse::parse_delivery("later").is_err());
    assert!(parse::parse_subagent_status("completed").is_ok());
    assert!(parse::parse_subagent_status("cancelled").is_ok());
    assert!(parse::parse_subagent_status("vanished").is_err());
    assert!(parse::parse_event_kind("text_delta").is_ok());
    assert!(parse::parse_event_kind("mystery").is_err());
    assert!(parse::parse_role("tool").is_ok());
    assert!(parse::parse_role("robot").is_err());
}

#[test]
fn parse_session_maps_every_live_column() {
    let row = vec![
        serde_json::json!("sess-1"),
        serde_json::Value::Null, // kind
        serde_json::json!("t"),
        serde_json::json!("build"),
        serde_json::json!("prov/m"),
        serde_json::json!("ap"),
        serde_json::json!("wh"),
        serde_json::json!(5),
        serde_json::json!(6),
        serde_json::json!("sum"),
        serde_json::json!(7),
        serde_json::json!(r#"["data:image/png;base64,xx"]"#),
        serde_json::json!(8),
        serde_json::json!("plan"),
        serde_json::json!("skill-x"),
        serde_json::json!("parent"),
        serde_json::json!("req"),
    ];
    let parsed = parse::parse_session(&row, "sess-1").unwrap();
    let meta = parsed.meta;
    assert_eq!(meta.id, "sess-1");
    assert_eq!(meta.kind, None);
    assert_eq!(meta.title.as_deref(), Some("t"));
    assert_eq!(meta.agent.as_deref(), Some("build"));
    assert_eq!(meta.autopilot_mode.as_deref(), Some("ap"));
    assert_eq!(meta.workdir_hash.as_deref(), Some("wh"));
    assert_eq!((meta.created_at, meta.updated_at), (5, 6));
    assert_eq!(meta.summary.as_deref(), Some("sum"));
    assert_eq!(meta.summary_seq, Some(7));
    assert_eq!(meta.summary_images.len(), 1);
    assert_eq!(meta.handoff_seq, Some(8));
    assert_eq!(meta.handoff_plan.as_deref(), Some("plan"));
    assert_eq!(meta.skill.as_deref(), Some("skill-x"));
    assert_eq!(meta.task_type.as_deref(), Some("parent"));
    assert_eq!(meta.requirement.as_deref(), Some("req"));
}

#[test]
fn parse_session_rejects_malformed_summary_images() {
    let mut row = vec![serde_json::json!("s")];
    row.extend(std::iter::repeat_n(serde_json::Value::Null, 16));
    row[11] = serde_json::json!("not json");
    assert!(parse::parse_session(&row, "s").is_err());
}

#[test]
fn legacy_event_and_input_rows_parse_strictly() {
    let event_row = vec![
        serde_json::json!("s1"),
        serde_json::json!("done"),
        serde_json::json!(r#"{"ok":true}"#),
        serde_json::json!("message.done"),
        serde_json::json!(42),
    ];
    let event: LegacyEvent = parse::parse_event(&event_row).unwrap();
    assert_eq!(event.record.ts, 42);
    assert_eq!(event.record.seq, None, "fresh ordinal on import");

    let input_row = vec![
        serde_json::json!("s1"),
        serde_json::json!("in-1"),
        serde_json::json!("queue"),
        serde_json::json!("next"),
        serde_json::json!("[]"),
        serde_json::Value::Null,
        serde_json::json!(1),
        serde_json::json!(2),
    ];
    let input: LegacyInput = parse::parse_input(&input_row).unwrap();
    assert_eq!(input.input.id, "in-1");
    assert_eq!(input.input.promoted_seq, Some(2));

    let bad = vec![
        serde_json::json!("s1"),
        serde_json::json!("in-1"),
        serde_json::json!("queue"),
        serde_json::json!("next"),
        serde_json::json!("{"),
        serde_json::Value::Null,
        serde_json::json!(1),
        serde_json::Value::Null,
    ];
    assert!(parse::parse_input(&bad).is_err());
}
