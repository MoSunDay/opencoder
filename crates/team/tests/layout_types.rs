//! Layout validation (traversal hardening), directory listing filters, and
//! the tolerant decision-JSON parser.

mod common;

use common::*;
use opencoder_team::types::{parse_decision, ClosingDecision, PlanDecision, ProfileDecision};
use opencoder_team::{
    fs_store, layout, validate_member, validate_sub_turn, validate_team_name, validate_topic_id,
    validate_turn,
};
use std::path::Path;
use ulid::Ulid;

#[test]
fn team_name_rules() {
    for ok in ["a", "a1", "team-x", "0", &"x".repeat(64)] {
        assert!(validate_team_name(ok), "{ok:?}");
    }
    for bad in [
        "",
        "A",
        "-a",
        "a_b",
        "a b",
        "a/b",
        "../etc",
        ".",
        "a..b",
        "é",
        &"x".repeat(65),
        "UPPER",
    ] {
        assert!(!validate_team_name(bad), "{bad:?}");
    }
}

#[test]
fn topic_id_must_be_ulid() {
    assert!(validate_topic_id(&Ulid::new().to_string()));
    for bad in [
        "",
        "../etc",
        "01ABC",
        "not-a-ulid",
        "01AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    ] {
        assert!(!validate_topic_id(bad), "{bad:?}");
    }
}

#[test]
fn turn_and_sub_turn_bounds() {
    assert!(!validate_turn(0));
    assert!(validate_turn(1));
    assert!(validate_turn(999));
    assert!(!validate_turn(1000));
    assert!(validate_sub_turn(0));
    assert!(validate_sub_turn(999));
    assert!(!validate_sub_turn(1000));
}

#[test]
fn member_id_rules() {
    assert!(validate_member("01ABCDEFGHJKLMNPQRSTVWXYZ0"));
    assert!(validate_member("node-1_x"));
    for bad in ["", "..", "a/b", "a b", &"x".repeat(65)] {
        assert!(!validate_member(bad), "{bad:?}");
    }
}

#[test]
fn path_constructors_reject_traversal_before_building() {
    let root = Path::new("/nfs/team");
    assert!(layout::team_dir(root, "../etc").is_err());
    assert!(layout::team_dir(root, "Demo").is_err());
    assert!(layout::topic_dir(root, "demo", "01ABC").is_err());
    assert!(layout::plan_file(root, "demo", &Ulid::new().to_string(), 0).is_err());
    assert!(layout::plan_file(root, "demo", &Ulid::new().to_string(), 1000).is_err());
    let topic = Ulid::new().to_string();
    assert!(layout::result_file(root, "demo", &topic, 1, 0, "../escape").is_err());
    assert!(layout::result_file(root, "demo", &topic, 1, 0, "a/b").is_err());
    assert!(layout::sub_dir(root, "demo", &topic, 1, 1000).is_err());
    // The good shapes stay inside the root.
    assert_eq!(
        layout::result_file(root, "demo", &topic, 1, 0, "01J").unwrap(),
        root.join("demo")
            .join(&topic)
            .join("1")
            .join("0")
            .join("01J")
            .join("result.json")
    );
    assert_eq!(
        layout::summary_file(root, "demo", &topic, 2, 1).unwrap(),
        root.join("demo")
            .join(&topic)
            .join("2")
            .join("1")
            .join("summary.json")
    );
}

#[test]
fn list_dirs_filter_invalid_names() {
    let root = tempfile::tempdir().unwrap();
    let team = root.path().join("demo");
    std::fs::create_dir_all(team.join("01AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")).unwrap(); // invalid: 27 chars
    let topic_a = team.join(Ulid::new().to_string());
    let topic_b = team.join(Ulid::new().to_string());
    std::fs::create_dir_all(&topic_a).unwrap();
    std::fs::create_dir_all(&topic_b).unwrap();
    std::fs::create_dir_all(team.join("notes.txt")).unwrap();
    let listed = layout::list_topic_dirs(&team).unwrap();
    let mut expected = vec![
        topic_a.file_name().unwrap().to_string_lossy().to_string(),
        topic_b.file_name().unwrap().to_string_lossy().to_string(),
    ];
    expected.sort();
    assert_eq!(listed, expected);

    std::fs::create_dir_all(root.path().join("other")).unwrap();
    std::fs::create_dir_all(root.path().join("Bad")).unwrap();
    std::fs::create_dir_all(root.path().join("a..b")).unwrap();
    assert_eq!(
        layout::list_team_dirs(root.path()).unwrap(),
        vec!["demo", "other"]
    );
    assert_eq!(
        layout::list_team_dirs(&root.path().join("missing")).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn parse_decision_accepts_raw_fence_and_noise() {
    let raw = r#"{"question":"q","participants":["n1"],"rationale":"r"}"#;
    let plan: PlanDecision = parse_decision(raw).unwrap();
    assert_eq!(plan.question, "q");

    let fenced = format!("好的，我的决策：\n```json\n{raw}\n```\n以上。");
    let plan: PlanDecision = parse_decision(&fenced).unwrap();
    assert_eq!(plan.participants, vec!["n1"]);

    let bare: PlanDecision = parse_decision(&format!("```\n{raw}\n```")).unwrap();
    assert_eq!(bare.rationale, "r");

    let noisy: PlanDecision = parse_decision(&format!("我认为决策如下 {raw} 请查收")).unwrap();
    assert_eq!(noisy.participants, vec!["n1"]);
}

#[test]
fn parse_decision_rejects_garbage() {
    for bad in [
        "我认为无法回答",
        "```json\n{\"a\":",
        "```json\n{}\n``` 然后再 ```json\n{}\n```",
        "[1,2,3]",
    ] {
        assert!(parse_decision::<PlanDecision>(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn parse_decision_field_shapes() {
    let closing: ClosingDecision =
        parse_decision(r#"{"complete":false,"next_question":"下一问","final_summary":null}"#)
            .unwrap();
    assert!(!closing.complete);
    let profile: ProfileDecision = parse_decision(r#"{"capabilities":["Rust","SQLite"]}"#).unwrap();
    assert_eq!(profile.capabilities.len(), 2);
    // serde(default) tolerance: unknown/missing optional fields parse.
    let closing: ClosingDecision = parse_decision("{}").unwrap();
    assert!(!closing.complete && closing.final_summary.is_none());
}

#[tokio::test]
async fn fs_store_team_lifecycle_and_listing_skips_broken() {
    let fx = fixture(2, 1).await;
    let (captain, members) = make_team(&fx, 1).await;

    assert!(
        fs_store::create_team(fx.root(), &team_meta(&captain, &members)).is_err(),
        "duplicate name"
    );

    // A dir with a valid name but corrupt team.json must not break listing.
    let broken = fx.root().join("broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("team.json"), "{ not json").unwrap();
    let teams = fs_store::list_teams(fx.root());
    assert_eq!(teams.len(), 1);
    assert_eq!(teams[0].name, "demo");

    let loaded = fs_store::load_team(fx.root(), "demo").unwrap();
    assert_eq!(loaded.captain.node_id, captain.id);
    assert!(fs_store::load_team(fx.root(), "missing").is_err());
}

#[tokio::test]
async fn atomic_write_bounds_and_topic_tree_roundtrip() {
    let fx = fixture(2, 1).await;
    let (_captain, members) = make_team(&fx, 2).await;
    let topic_id = start(&fx, "讨论 A").await;

    let mut meta = fs_store::load_topic(fx.root(), "demo", &topic_id).unwrap();
    assert_eq!(meta.status, "executing");
    meta.status = "finished".into();
    meta.finish_reason = Some("complete".into());
    fs_store::save_topic(fx.root(), &meta).unwrap();

    let plan = opencoder_team::types::PlanRecord {
        turn: 1,
        question: "q1".into(),
        participants: members.iter().map(|m| m.id.clone()).collect(),
        rationale: "why".into(),
    };
    fs_store::write_plan(fx.root(), "demo", &topic_id, &plan).unwrap();
    for m in &members {
        fs_store::write_result(
            fx.root(),
            "demo",
            &topic_id,
            &opencoder_team::types::ResultRecord {
                node_id: m.id.clone(),
                turn: 1,
                sub_turn: 0,
                kind: "answer".into(),
                answer: format!("ans-{}", m.id),
                ok: true,
                error: None,
                created_at: 5,
            },
        )
        .unwrap();
    }
    fs_store::write_summary(
        fx.root(),
        "demo",
        &topic_id,
        1,
        0,
        &opencoder_team::types::SummaryRecord {
            summary: "s0".into(),
            aligned: true,
            ambiguities: vec![],
            created_at: 6,
        },
    )
    .unwrap();

    let (tree_meta, turns) = fs_store::read_topic_tree(fx.root(), "demo", &topic_id).unwrap();
    assert_eq!(tree_meta.status, "finished");
    assert_eq!(tree_meta.finish_reason.as_deref(), Some("complete"));
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].turn, 1);
    assert_eq!(turns[0].plan.as_ref().unwrap().question, "q1");
    assert_eq!(turns[0].sub_turns.len(), 1);
    assert_eq!(turns[0].sub_turns[0].results.len(), 2);
    assert!(turns[0].sub_turns[0].summary.as_ref().unwrap().aligned);

    // No stray tmp files survive atomic writes.
    let stray: Vec<_> = std::fs::read_dir(fx.root().join("demo").join(&topic_id).join("1"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
        .collect();
    assert!(stray.is_empty());

    // Size bounds: sub-minimum and over-maximum writes are refused.
    assert!(fs_store::atomic_write(&fx.root().join("x.json"), b"").is_err());
    let big = vec![b'x'; fs_store::MAX_FILE_BYTES + 1];
    assert!(fs_store::atomic_write(&fx.root().join("big.json"), &big).is_err());
    assert!(!fx.root().join("big.json").exists());
}
