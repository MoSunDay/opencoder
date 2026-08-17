use super::*;

#[test]
fn explicit_attach_target_bare_ts_returns_none() {
    // A bare `ts` (no --session) always creates a new session -- never attaches.
    assert_eq!(explicit_attach_target(None, false), None);
    assert_eq!(explicit_attach_target(None, true), None);
}

#[test]
fn explicit_attach_target_session_exists_attaches() {
    // `--session <id>` whose tmux session is live -> attach to it.
    assert_eq!(
        explicit_attach_target(Some("01ABCD"), true),
        Some(session_name("01ABCD"))
    );
}

#[test]
fn explicit_attach_target_session_not_live_returns_none() {
    // `--session <id>` but the tmux session is dead -> create fresh.
    assert_eq!(explicit_attach_target(Some("01ABCD"), false), None);
}

#[test]
fn bare_ts_always_builds_new_tmux_session_command() {
    let exe = Path::new("/bin/opencoder");
    let workdir = Path::new("/work/repo");
    let outside = spawn_args(exe, workdir, "01ABC", false);
    let inside = spawn_args(exe, workdir, "01ABC", true);
    assert_eq!(outside[0], "new-session");
    assert!(!outside.iter().any(|arg| arg == "-d"));
    assert_eq!(inside[0], "new-session");
    assert!(inside.iter().any(|arg| arg == "-d"));
    assert!(inside.iter().any(|arg| arg == "opencode-01ABC"));
    assert!(inside.iter().any(|arg| arg == "/work/repo"));
}

#[test]
fn managed_target_resolves_list_prefix_full_id_and_tmux_index() {
    let full_id = "01ABCDEFGHJKMNPQRSTVWXYZ12";
    let mut managed = mk_managed(full_id, 0);
    managed.tmux_id = "$7".into();
    let tmux = vec![managed];
    assert_eq!(resolve_managed_id("01ABCDEF", &tmux, &[]).unwrap(), full_id);
    assert_eq!(resolve_managed_id(full_id, &tmux, &[]).unwrap(), full_id);
    assert_eq!(
        resolve_managed_id(&format!("opencode-{full_id}"), &tmux, &[]).unwrap(),
        full_id
    );
    assert_eq!(resolve_managed_id("$7", &tmux, &[]).unwrap(), full_id);
    assert!(resolve_managed_id("$8", &tmux, &[]).is_err());
}

#[test]
fn managed_target_rejects_ambiguous_prefix() {
    let tmux = vec![
        mk_managed("01ABCDEF111111111111111111", 0),
        mk_managed("01ABCDEF222222222222222222", 0),
    ];
    let error = resolve_managed_id("01ABCDEF", &tmux, &[]).unwrap_err();
    assert!(error.to_string().contains("ambiguous"));
}

#[test]
fn managed_target_resolves_stopped_registry_prefix() {
    let full_id = "01STORE0ABCDEFGHJKMNPQRSTV";
    let records = vec![mk_record(full_id, Some("/work/project"), "task", None, 1)];
    assert_eq!(
        resolve_managed_id("01STORE0", &[], &records).unwrap(),
        full_id
    );
}

#[test]
fn list_legend_has_no_removed_flags() {
    // The `--new` flag was removed: a bare `ts` always creates, so the
    // legend must not advertise it. Advertising a dead command is a
    // silent regression this guard catches.
    assert!(
        !LIST_LEGEND.contains("--new"),
        "legend must not reference removed --new: {LIST_LEGEND}"
    );
    assert!(
        LIST_LEGEND.contains("resume"),
        "must advertise resume: {LIST_LEGEND}"
    );
    assert!(
        LIST_LEGEND.contains("clean"),
        "must advertise clean: {LIST_LEGEND}"
    );
    assert!(
        LIST_LEGEND.contains("delete"),
        "must advertise delete: {LIST_LEGEND}"
    );
}

fn mk_managed_at(id: &str, attached: u8, pane_path: &str, created: i64) -> ManagedSession {
    ManagedSession {
        name: session_name(id),
        tmux_id: "$0".into(),
        created,
        attached,
        pane_path: pane_path.into(),
    }
}

fn mk_managed(id: &str, attached: u8) -> ManagedSession {
    mk_managed_at(id, attached, "/root/proj", 0)
}

/// A registry row for build_rows/cleanup tests. Registry rows are ts
/// sessions by construction — the old `agent`/`model` seed distinction
/// lives in the producers (ts_start register + tui mirror), not here.
fn mk_record(
    id: &str,
    workdir: Option<&str>,
    preview: &str,
    title: Option<&str>,
    created: i64,
) -> TsRecord {
    TsRecord {
        id: id.to_string(),
        workdir: workdir.map(PathBuf::from),
        store_dir: Some(PathBuf::from("/data/store")),
        created_at: created,
        updated_at: created,
        title: title.map(String::from),
        preview: preview.to_string(),
    }
}

#[test]
fn classify_three_states() {
    let m1 = mk_managed("01AAA", 1);
    let m2 = mk_managed("02BBB", 0);
    let map: HashMap<String, &ManagedSession> =
        [("01AAA".to_string(), &m1), ("02BBB".to_string(), &m2)]
            .into_iter()
            .collect();

    assert_eq!(classify("01AAA", &map), TmuxState::Attached);
    assert_eq!(classify("02BBB", &map), TmuxState::Detached);
    assert_eq!(classify("NOTHERE", &map), TmuxState::Dead);
}

fn row(id: &str, path: &str, created: i64, state: TmuxState) -> GlobalRow {
    GlobalRow {
        id: id.to_string(),
        path: path.to_string(),
        created_at: created,
        state,
        task: String::new(),
    }
}

#[test]
fn sort_by_path_then_created_desc() {
    let mut rows = vec![
        row("b", "~/projB", 100, TmuxState::Attached),
        row("a1", "~/projA", 300, TmuxState::Detached),
        row("a2", "~/projA", 400, TmuxState::Dead),
        row("a0", "~/projA", 300, TmuxState::Attached),
    ];
    sort_rows(&mut rows);
    assert_eq!(rows[0].id, "a2");
    assert_eq!(rows[1].id, "a0");
    assert_eq!(rows[2].id, "a1");
    assert_eq!(rows[3].id, "b");
}

#[test]
fn build_rows_unions_live_tmux_and_registered_stopped() {
    let records = vec![
        // Registered ts session, used, currently live in tmux -> enriched
        // live row with the tmux path.
        mk_record("AA1", Some("/work/projY"), "build the api", Some("t"), 200),
        // Registered ts session, used, tmux dead -> stopped row.
        mk_record("EE5", Some("/work/projY"), "refactor module", None, 150),
        // Title-only row (mode-switched session without a preview) -> still
        // counts as started and is listed.
        mk_record("TT4", Some("/work/projY"), "", Some("titled task"), 120),
        // Never-started registration-time seed (no preview/title) -> never
        // listed. Plain tui/run sessions cannot appear in the registry.
        mk_record("CC3", None, "", None, 50),
    ];
    let tmux = vec![
        mk_managed_at("AA1", 1, "/work/projY", 2),
        mk_managed_at("DD1", 0, "/work/projX", 1),
    ];

    let rows = build_rows(&records, &tmux);

    assert_eq!(
        rows.len(),
        4,
        "AA1 live + DD1 live + EE5/TT4 stopped; CC3 excluded"
    );
    let aa = rows
        .iter()
        .find(|r| r.id == "AA1")
        .expect("live enriched row");
    assert_eq!(aa.state, TmuxState::Attached);
    assert_eq!(
        aa.path, "/work/projY",
        "live path comes from tmux pane_current_path"
    );
    assert_eq!(
        aa.task, "build the api",
        "task enriched from the registry row"
    );
    assert_eq!(
        aa.created_at, 200,
        "creation time taken from the registry (ms)"
    );
    // Dedupe: AA1 appears exactly once (tmux row wins over a stopped row).
    assert_eq!(rows.iter().filter(|r| r.id == "AA1").count(), 1);

    let dd = rows.iter().find(|r| r.id == "DD1").expect("tmux-only row");
    assert_eq!(dd.state, TmuxState::Detached);
    assert_eq!(dd.path, "/work/projX");
    assert_eq!(
        dd.created_at, 1000,
        "tmux created (unix seconds) scaled to ms"
    );
    assert_eq!(dd.task, "(no task yet)", "no registry row -> placeholder");

    let ee = rows.iter().find(|r| r.id == "EE5").expect("stopped row");
    assert_eq!(ee.state, TmuxState::Dead);
    assert_eq!(
        ee.path, "/work/projY",
        "stopped path from the recorded workdir"
    );
    assert_eq!(ee.task, "refactor module");

    let tt = rows.iter().find(|r| r.id == "TT4").expect("title-only row");
    assert_eq!(tt.state, TmuxState::Dead);
    assert_eq!(
        tt.task, "titled task",
        "title-only sessions still get a task head"
    );
    assert!(
        rows.iter().all(|r| r.id != "CC3"),
        "empty seed never listed"
    );
}

#[test]
fn cleanup_targets_are_dead_registry_rows_grouped_by_store() {
    let records = vec![
        // Dead used session -> targeted.
        mk_record("DEAD_A", Some("/work/a"), "used", None, 4),
        // Dead empty seed: the list hides it, but cleanup still removes it
        // (the store row exists and would otherwise leak forever).
        mk_record("EMPTY_B", Some("/work/b"), "", None, 3),
        // Live session -> never targeted.
        mk_record("LIVE_C", Some("/work/c"), "live", None, 2),
    ];
    let live_ids = HashSet::from(["LIVE_C"]);
    // No owning store dir -> cannot be grouped (unregistered separately
    // by ts_cleanup).
    let mut no_dir = mk_record("NO_DIR_D", None, "content", None, 1);
    no_dir.store_dir = None;
    let records = [records, vec![no_dir]].concat();

    let targets = cleanup_targets(&records, &live_ids);

    assert_eq!(
        targets,
        BTreeMap::from([(
            PathBuf::from("/data/store"),
            vec!["DEAD_A".to_string(), "EMPTY_B".to_string()]
        ),]),
        "both records share the mk_record store dir"
    );
}

#[test]
fn now_ms_is_milliseconds() {
    let t = opencoder_core::message::now_ms();
    assert!(
        t > 1_000_000_000_000,
        "now_ms should be in milliseconds, got {t}"
    );
}
