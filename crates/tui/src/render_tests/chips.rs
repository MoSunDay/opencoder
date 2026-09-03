use super::*;
use crate::chat::{ChatBlock, ChatView};
use crate::theme::{agent_chip_fg, mode_flash_bg};
use opencoder_session::SessionEvent;

/// Issue #6: the `[agent]` status chip is Yellow in plan (read-only) mode
/// and Cyan for every other agent. Guards against a regression to the old
/// uniform Magenta.
#[test]
fn agent_chip_color_is_yellow_for_plan_cyan_otherwise() {
    assert_eq!(agent_chip_fg("plan"), Color::Yellow);
    assert_eq!(agent_chip_fg("act"), Color::Cyan);
    // The interlude `sandbox` spelling is gone; it must not map to the
    // plan hue anymore.
    assert_eq!(agent_chip_fg("sandbox"), Color::Cyan);
    assert_eq!(agent_chip_fg("explore"), Color::Cyan);
    assert_eq!(agent_chip_fg(""), Color::Cyan);
}

/// Issue #6: the plan/act mode-flash chip background is Yellow for
/// plan, Cyan for act. Both the agent chip and the flash share the same
/// theme mapping, so they never visually disagree.
#[test]
fn mode_flash_bg_matches_plan_yellow_act_cyan() {
    assert_eq!(mode_flash_bg(true), Color::Yellow);
    assert_eq!(mode_flash_bg(false), Color::Cyan);
    // The two theme helpers agree on plan/act, so the chip and flash
    // always render the same hue.
    assert_eq!(agent_chip_fg("plan"), mode_flash_bg(true));
    assert_eq!(agent_chip_fg("act"), mode_flash_bg(false));
}

/// Issue #5 core invariant: while a preamble block is WITHHELD (multiple
/// subagents running), the `header_line_idx` values reported by
/// `thinking_headers()` and `subagent_headers()` must exactly match the
/// line indices in `flatten_with()` where those headers actually render.
/// If any of the `is_withheld` guards in those three functions drift out
/// of sync, a header index would point at the wrong row and mouse clicks
/// would land on the wrong block.
#[test]
fn header_line_indices_aligned_with_flatten_while_withheld() {
    let mut v = ChatView::default();
    // Preamble assistant text — withheld once 2 subagents run. Its "say:"
    // header + 2 content lines mean a stale (non-skipping) accounting
    // would shift every later header by 3 rows.
    v.apply(&SessionEvent::TextDelta(
        "preamble line one\npreamble line two".into(),
    ));
    v.apply(&SessionEvent::SubagentStart {
        id: "a".into(),
        kind: "explore".into(),
        prompt: "pa".into(),
        child_session_id: "ca".into(),
    });
    v.apply(&SessionEvent::SubagentStart {
        id: "b".into(),
        kind: "explore".into(),
        prompt: "pb".into(),
        child_session_id: "cb".into(),
    });
    // Thinking block after the subagents: its header_line_idx is the
    // canary — if the withheld preamble were counted it would overshoot.
    // Legacy shape built directly (live reasoning goes into the ladder):
    // the header canary keeps guarding the withheld-preamble accounting.
    v.blocks.push(ChatBlock::Thinking {
        text: "post\ndispatch\nanalysis".into(),
        collapsed: true,
        sealed: true,
    });

    assert!(
        v.hidden_assistant_idx.is_some(),
        "preamble must be withheld"
    );
    assert_eq!(v.subagents_running, 2);
    let flat = v.flatten_with(0, 0);

    let line_text =
        |idx: usize| -> String { flat[idx].spans.iter().map(|s| s.content.clone()).collect() };
    // Every thinking header points at a flatten line containing "Thinking".
    let th = v.thinking_headers();
    assert!(!th.is_empty());
    for h in &th {
        let txt = line_text(h.header_line_idx);
        assert!(
            txt.contains("Thinking"),
            "thinking header_line_idx {} -> {:?}",
            h.header_line_idx,
            txt,
        );
    }
    // Every subagent header points at a flatten line containing "subagent".
    let sh = v.subagent_headers();
    assert_eq!(sh.len(), 2);
    for h in &sh {
        let txt = line_text(h.header_line_idx);
        assert!(
            txt.contains("subagent"),
            "subagent header_line_idx {} -> {:?}",
            h.header_line_idx,
            txt,
        );
    }
    // No two headers collide on the same rendered line.
    let mut all_idx: Vec<usize> = th.iter().map(|h| h.header_line_idx).collect();
    all_idx.extend(sh.iter().map(|h| h.header_line_idx));
    let mut sorted = all_idx.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), all_idx.len(), "collide: {:?}", all_idx);
    // The withheld preamble contributes ZERO lines to flatten.
    for (i, line) in flat.iter().enumerate() {
        let txt: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(
            !txt.contains("preamble line"),
            "line {i}: withheld preamble leaked: {:?}",
            txt,
        );
    }
}

/// The autopilot status chip is tri-state: `AP` for fully-automatic mode,
/// `RV` for auto-review, and absent when off. Chips draw on the local
/// (magenta) background at the composer top-right.
#[test]
fn ap_chip_reflects_autopilot_mode() {
    use crate::render::render;
    use opencoder_core::ApMode;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Render one frame with benign defaults and the given autopilot mode.
    fn draw(mode: ApMode, terminal: &mut Terminal<TestBackend>) {
        let chat = ChatView::default();
        let mut scroll = 0u32;
        let mut queue_scroll = 0u32;
        let mut hits = MouseHits::default();
        render(
            terminal,
            &chat,
            "",
            0,
            &Line::raw("title"),
            false,
            0,
            0,
            200_000,
            200_000,
            "idle",
            &[],
            &[],
            &mut scroll,
            true,
            &mut queue_scroll,
            0,
            0,
            None, // mode_flash
            None, // skill_menu
            None, // task_picker
            None, // command_menu
            None, // file_menu
            None, // model_menu
            None, // mcp_menu
            None, // envs_menu
            None, // cli_menu
            None, // skill_toggle_menu
            None, // ap_menu
            None, // cache_salt_menu
            None, // keymap_menu
            None, // question_menu
            &mut hits,
            &mut None,
            false,
            false,
            &[],
            false,
            None,
            None,
            0,
            0,
            true,
            mode,
            "act",
            false,
            None,
        )
        .unwrap();
    }

    /// Whether any cell with one of `letters` sits on the local (magenta) bg.
    fn chip_cell(terminal: &mut Terminal<TestBackend>, letters: &[&str]) -> bool {
        let local = crate::theme::local_color();
        let buf = terminal.backend().buffer();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if let Some(cell) = buf.cell((x, y)) {
                    if letters.contains(&cell.symbol()) && cell.bg == local {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Rows whose flattened text contains `needle` (for full chip text).
    fn chip_rows(terminal: &mut Terminal<TestBackend>, needle: &str) -> Vec<u16> {
        let buf = terminal.backend().buffer();
        let area = buf.area;
        (0..area.height)
            .filter(|&y| row_text(buf, y, area.width).contains(needle))
            .collect()
    }

    // Off: no AP/RV chip anywhere on the frame.
    {
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        draw(ApMode::Off, &mut terminal);
        assert!(
            !chip_cell(&mut terminal, &["A", "P"]),
            "no AP chip letters on local bg when mode is off"
        );
        assert!(
            chip_rows(&mut terminal, " AP ").is_empty()
                && chip_rows(&mut terminal, " RV ").is_empty(),
            "neither AP nor RV chip text may render when mode is off"
        );
    }

    // Ap: the " AP " chip sits on a local-color (magenta) background cell.
    {
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        draw(ApMode::Ap, &mut terminal);
        assert!(
            chip_cell(&mut terminal, &["A", "P"]),
            "AP chip must render in ap mode"
        );
        assert!(
            !chip_rows(&mut terminal, " AP ").is_empty(),
            "AP chip text must be visible; rows with it exist"
        );
        assert!(
            chip_rows(&mut terminal, " RV ").is_empty(),
            "ap mode must not show an RV chip"
        );
    }

    // Review: the " RV " chip replaces AP.
    {
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        draw(ApMode::Review, &mut terminal);
        assert!(
            chip_cell(&mut terminal, &["R", "V"]),
            "RV chip must render in review mode"
        );
        assert!(
            !chip_rows(&mut terminal, " RV ").is_empty(),
            "RV chip text must be visible; rows with it exist"
        );
        assert!(
            chip_rows(&mut terminal, " AP ").is_empty(),
            "review mode must not show an AP chip"
        );
    }
}

/// Mode-flash chip colouring contract: ONLY the definite plan-family
/// flashes — "→ plan mode" (agent switch via /plan) and "→ edit plan"
/// (the plan-text editor, entered from the plan agent) — participate in
/// the two-colour scheme. Every other flash — the busy hint ("⏳ busy — mode
/// switch blocked, retry when idle"), "→ act mode", and any future neutral
/// text that merely CONTAINS "plan" — renders on the accent background.
/// Guards against the old `text.contains("plan")` substring guess
/// mis-tinting unrelated hints.
#[test]
fn mode_flash_chip_two_colour_only_for_definite_switch() {
    use crate::render::render;
    use crate::theme::{accent, warn_color};
    use opencoder_core::ApMode;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Render one frame with benign defaults and the given mode-flash text.
    fn draw(mode_flash: &str, terminal: &mut Terminal<TestBackend>) {
        let chat = ChatView::default();
        let mut scroll = 0u32;
        let mut queue_scroll = 0u32;
        let mut hits = MouseHits::default();
        render(
            terminal,
            &chat,
            "",
            0,
            &Line::raw("title"),
            false,
            0,
            0,
            200_000,
            200_000,
            "idle",
            &[],
            &[],
            &mut scroll,
            true,
            &mut queue_scroll,
            0,
            0,
            Some(mode_flash),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &mut hits,
            &mut None,
            false,
            false,
            &[],
            false,
            None,
            None,
            0,
            0,
            true,
            ApMode::Off,
            "act",
            false,
            None,
        )
        .unwrap();
    }

    /// Background colours present on the (unique) row containing `needle`.
    /// Needles deliberately avoid width-ambiguous leading glyphs (⏳/→ render
    /// as double-width cells whose trailing skip-cell injects an extra space
    /// into the concatenated row text).
    fn row_bgs(terminal: &Terminal<TestBackend>, needle: &str) -> Vec<Color> {
        let buf = terminal.backend().buffer();
        let area = buf.area;
        let mut rows = (0..area.height)
            .filter(|&y| row_text(buf, y, area.width).contains(needle))
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 1, "needle {needle:?} must hit exactly one row");
        let y = rows.remove(0);
        (0..area.width)
            .filter_map(|x| buf.cell((x, y)).map(|c| c.bg))
            .collect()
    }

    let check = |flash: &str, needle: &str, expect_plan: bool| {
        let mut terminal = Terminal::new(TestBackend::new(90, 24)).unwrap();
        draw(flash, &mut terminal);
        let bgs = row_bgs(&terminal, needle);
        let (want, other) = if expect_plan {
            (warn_color(), accent())
        } else {
            (accent(), warn_color())
        };
        assert!(
            bgs.contains(&want),
            "flash {flash:?} must render on {want:?} bg; got {bgs:?}"
        );
        assert!(
            !bgs.contains(&other),
            "flash {flash:?} must NOT render on {other:?} bg; got {bgs:?}"
        );
    };

    // Definite mode-switch flashes keep the two-colour scheme.
    check("\u{2192} plan mode", "plan mode", true);
    check("\u{2192} edit plan", "edit plan", true);
    check("\u{2192} act mode", "act mode", false);
    // Busy hint: accent — it is not a completed switch.
    check(
        "\u{23f3} busy \u{2014} mode switch blocked, retry when idle",
        "busy",
        false,
    );
    // Neutral future text that merely mentions "plan": accent, NOT plan
    // colour (the substring `contains("plan")` guess would tint this).
    check("plan submitted", "plan submitted", false);
}
