//! Transient skill-context tail reminder.
//!
//! Skill bodies no longer ship in the system prompt — appending them there
//! rewrote the payload's first bytes on every activation, destabilising
//! provider prompt-prefix caching. Instead every LLM call derives, from
//! session state alone, one synthetic user message appended at the END of
//! the request payload: never recorded to the store, never replayed. It
//! carries (a) a catalog of config-enabled skills (lazy-load hint for
//! `~/.opencoder/skills`) and (b) the active skill's source path (from the
//! `> Source:` prefix `body_with_source` writes).
//!
//! On top of the transient tail, [`ensure_full_body_loaded`] (called by
//! `run_loop` once per LLM round, after compaction) idempotently injects the
//! ACTIVE skill's path + body as ONE persistent `synthetic=true` user
//! message (`[skill loaded] <path>` marker), so the model no longer burns a
//! tool call reading the SKILL.md. Bodies over ~20K tokens are injected as a
//! whole-line prefix plus an `[INCOMPLETE SKILL]` continuation notice (same
//! style as the read tool's `[INCOMPLETE READ]`); the tail reminder then
//! only points back at that message as a fallback (e.g. after compaction
//! folded it into a summary, which triggers a fresh injection).

use std::path::Path;

use opencoder_core::{discover_in, skills_dir, AgentMode, Config, Message, Role};

use crate::SessionState;

/// Extract the source-file path from a `> Source: <path>` prefix line
/// (written by `opencoder_core::body_with_source`). `None` when absent or
/// the path is empty.
pub fn source_path_from_body(body: &str) -> Option<&str> {
    let rest = body.strip_prefix("> Source: ")?;
    let path = rest.split('\n').next()?.trim();
    (!path.is_empty()).then_some(path)
}

/// Enabled-skill catalog for an explicit discovery root: the intersection of
/// `config.enabled_skill_names()` with `discover_in(root)`, sorted by name.
pub fn catalog_entries_in(root: &Path, config: &Config) -> Vec<(String, String)> {
    let enabled = config.enabled_skill_names();
    if enabled.is_empty() {
        return Vec::new();
    }
    discover_in(root)
        .into_iter()
        .filter(|s| enabled.iter().any(|n| n == &s.name))
        .map(|s| (s.name, s.description))
        .collect()
}

/// Enabled-skill catalog for the default skills directory.
pub fn catalog_entries(config: &Config) -> Vec<(String, String)> {
    catalog_entries_in(&skills_dir(), config)
}

/// Pure builder for the reminder text. Empty string when both inputs are
/// empty; otherwise the `[skills]` and/or `[active skill]` sections joined
/// by a blank line.
pub fn reminder_text(catalog: &[(String, String)], active_path: Option<&str>) -> String {
    let mut sections: Vec<String> = Vec::new();
    if !catalog.is_empty() {
        let mut lines = vec![format!(
            "[skills]\nEnabled skills live under {}:",
            skills_dir().display()
        )];
        for (name, description) in catalog {
            lines.push(match description.as_str() {
                "" => format!("- {name}"),
                d => format!("- {name}: {d}"),
            });
        }
        lines.push(
            "When a task matches an enabled skill, read its SKILL.md file \
             under that directory first, then follow it."
                .into(),
        );
        sections.push(lines.join("\n"));
    }
    if let Some(path) = active_path {
        sections.push(format!(
            "[active skill]\nAn active skill is in effect: {path}\n\
             Its full body (or a truncated version) is loaded into the conversation above \
             as a `[skill loaded]` message; if you cannot find it there (e.g. after \
             compaction), read that file, then follow it."
        ));
    }
    sections.join("\n\n")
}

/// Build the per-call tail reminder message, or `None` when there is nothing
/// to remind about. Only Primary agents get skill context: subagents run
/// scoped tasks, and `workflow` — although itself a Primary-mode agent (see
/// `crates/core/src/agent.rs`) — is the todos scheduler and must not receive
/// it, hence the explicit name check.
pub fn tail_reminder(session: &SessionState) -> Option<Message> {
    if session.agent.mode != AgentMode::Primary || session.agent.name == "workflow" {
        return None;
    }
    let skill_prompt = session.skill_prompt_cloned();
    let active_path = skill_prompt.as_deref().and_then(source_path_from_body);
    let text = reminder_text(&catalog_entries(&session.config), active_path);
    if text.is_empty() {
        return None;
    }
    let mut msg = Message::user(crate::runner::new_id(), text);
    msg.synthetic = true;
    Some(msg)
}

/// Token budget for one injected skill body, in `opencoder_llm::estimate`
/// tokens — the same unit the read tool's 5K cap and compaction thresholds
/// use. Skills are standing instructions, so the budget is scaled up; past
/// it the body ships as a line-truncated prefix plus a continuation notice.
const MAX_INJECT_TOKENS: usize = 20_000;

/// Marker line of the persistent full-body message: `[skill loaded] <path>`.
pub fn full_body_marker(path: &str) -> String {
    format!("[skill loaded] {path}")
}

/// Whether the transcript already carries the `[skill loaded]` message for
/// `path` — the idempotence gate for [`ensure_full_body_loaded`] (same
/// marker-scan precedent as the compaction summary). Matching requires the
/// newline after the path so `/a/SKILL.md` never matches a marker written
/// for `/a/SKILL.md.bak`.
pub fn loaded_marker_matches(messages: &[Message], path: &str) -> bool {
    let marker = format!("{}\n", full_body_marker(path));
    messages
        .iter()
        .any(|m| m.synthetic && m.role == Role::User && m.text().starts_with(&marker))
}

/// Body to inject for a skill sourced at `path`: verbatim when within
/// budget, else the largest whole-line prefix that fits plus an
/// `[INCOMPLETE SKILL]` notice whose `offset=` is the 1-based line right
/// after the truncation point — aligned with the read tool's `offset`
/// semantics and mirroring its `[INCOMPLETE READ]` style.
pub fn injectable_body(body: &str, path: &str) -> String {
    if opencoder_llm::estimate(body) <= MAX_INJECT_TOKENS {
        return body.to_string();
    }
    let lines: Vec<&str> = body.lines().collect();
    // `estimate` grows monotonically with the kept-line count, so
    // binary-search the largest prefix within budget (a handful of joins
    // instead of one per line).
    let fits = |k: usize| opencoder_llm::estimate(&lines[..k].join("\n")) <= MAX_INJECT_TOKENS;
    let (mut lo, mut hi) = (0usize, lines.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let remaining = lines.len() - lo;
    let next_line = lo + 1;
    let notice = format!(
        "[INCOMPLETE SKILL] truncated at ~20K tokens; {remaining} lines remain; \
         read the rest with the read tool: read(path=\"{path}\", offset={next_line})."
    );
    match lo {
        0 => notice,
        k => format!("{}\n{}", lines[..k].join("\n"), notice),
    }
}

/// Idempotently inject the active skill's body into the PERSISTENT
/// transcript as one `synthetic=true` user message (marker line + blank
/// line + [`injectable_body`] output), recorded via `SessionState::record`
/// so it survives resume. Called by `run_loop` after the compaction check
/// and before every LLM round; the marker scan keeps it one-shot per skill
/// path, and compaction folding the message into a summary simply triggers
/// a fresh (possibly truncated) injection next round. Gating matches
/// [`tail_reminder`]: Primary agents only, `workflow` excluded, and a body
/// without a `> Source:` prefix (legacy) yields no path and no injection.
pub async fn ensure_full_body_loaded(session: &mut SessionState) {
    if session.agent.mode != AgentMode::Primary || session.agent.name == "workflow" {
        return;
    }
    let prompt = session.skill_prompt_cloned();
    let Some(prompt) = prompt.as_deref() else {
        return;
    };
    let Some(path) = source_path_from_body(prompt) else {
        return;
    };
    if loaded_marker_matches(&session.messages, path) {
        return;
    }
    // Strip the `> Source: <path>` prefix block: the marker already carries
    // the path, and the injected lines must mirror the file's own body so
    // the truncation `offset=` matches what `read(path, offset=…)` returns.
    let body = prompt
        .strip_prefix("> Source: ")
        .and_then(|rest| rest.split_once("\n\n"))
        .map(|(_, body)| body)
        .unwrap_or(prompt);
    // A skill whose parsed body is empty (frontmatter-only file) carries
    // nothing beyond the path the transient tail already points at;
    // injecting would record a marker-only message.
    if body.trim().is_empty() {
        return;
    }
    let text = format!(
        "{}\n\n{}",
        full_body_marker(path),
        injectable_body(body, path)
    );
    let mut msg = Message::user(crate::runner::new_id(), text);
    msg.synthetic = true;
    session.record(msg).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::config::SkillConfig;
    use opencoder_core::{resolve_agent, Role};
    use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
    use std::sync::Arc;

    fn client() -> Arc<dyn ChatStream> {
        Arc::new(
            MockChatClient::new().with_default(vec![LlmEvent::Completed {
                text: "ok".into(),
                tool_calls: vec![],
                usage: None,
            }]),
        )
    }

    #[test]
    fn source_path_from_body_variants() {
        let body = "> Source: /skills/review/SKILL.md\n\nbody";
        assert_eq!(source_path_from_body(body), Some("/skills/review/SKILL.md"));
        assert_eq!(source_path_from_body("no prefix"), None);
        assert_eq!(source_path_from_body("> Source: \n\nb"), None);
    }

    #[test]
    fn catalog_entries_in_intersects_and_sorts() {
        let dir = tempfile::tempdir().unwrap();
        for (f, front) in [
            ("beta.md", "---\nname: beta\ndescription: B\n---\nb"),
            ("alpha.md", "---\nname: alpha\ndescription: A\n---\na"),
        ] {
            std::fs::write(dir.path().join(f), front).unwrap();
        }
        let mut cfg = Config::default();
        assert!(
            catalog_entries_in(dir.path(), &cfg).is_empty(),
            "nothing enabled"
        );
        let sc = SkillConfig { enabled: true };
        cfg.skills.insert("beta".into(), sc);
        let got = catalog_entries_in(dir.path(), &cfg);
        assert_eq!(
            got,
            vec![("beta".into(), "B".into())],
            "enabled only, by name"
        );
    }

    #[test]
    fn reminder_text_sections() {
        assert_eq!(reminder_text(&[], None), "", "neither section -> empty");
        let catalog = vec![
            ("alpha".into(), "A skill".into()),
            ("plain".into(), String::new()),
        ];
        let both = reminder_text(&catalog, Some("/skills/x/SKILL.md"));
        let head = "[skills]\nEnabled skills live under";
        assert!(
            both.starts_with(head) && both.contains("- alpha: A skill\n- plain\n"),
            "{both}"
        );
        assert!(
            both.contains("read its SKILL.md file"),
            "guidance line: {both}"
        );
        let active = "\n\n[active skill]\nAn active skill is in effect: /skills/x/SKILL.md";
        assert!(
            both.contains(active),
            "sections joined by a blank line: {both}"
        );
        assert!(
            both.contains("as a `[skill loaded]` message"),
            "activation section points at the loaded message: {both}"
        );
        assert!(
            both.contains("read that file, then follow it"),
            "read fallback retained: {both}"
        );
        let catalog_only = reminder_text(&catalog, None);
        assert!(catalog_only.contains("[skills]") && !catalog_only.contains("[active skill]"));
        let active_only = reminder_text(&[], Some("/p"));
        assert!(active_only.starts_with("[active skill]\n"), "{active_only}");
    }

    #[test]
    fn tail_reminder_gating_and_content() {
        let mk = |name: &str| {
            SessionState::new(
                "t",
                resolve_agent(name).unwrap(),
                Config::default(),
                client(),
                Path::new("/tmp").into(),
            )
        };
        let s = mk("act");
        assert!(tail_reminder(&s).is_none(), "nothing to remind about");
        s.set_skill(Some("> Source: /skills/rev/SKILL.md\n\nREV".into()));
        let msg = tail_reminder(&s).expect("primary + Source prefix");
        assert_eq!(msg.role, Role::User);
        assert!(msg.synthetic, "transient, never recorded");
        let text = msg.text();
        assert!(text.contains("[active skill]") && text.contains("/skills/rev/SKILL.md"));
        assert!(
            tail_reminder(&mk("workflow")).is_none(),
            "workflow excluded"
        );
        assert!(tail_reminder(&mk("explore")).is_none(), "subagent excluded");
    }

    #[test]
    fn injectable_body_small_is_verbatim() {
        let out = injectable_body("line1\nline2", "/s/SKILL.md");
        assert_eq!(out, "line1\nline2", "within budget -> unchanged");
    }

    #[test]
    fn injectable_body_exactly_at_budget_is_verbatim() {
        // 80,000 chars / 4 = exactly 20,000 tokens: at the limit, not over.
        let body = "a".repeat(80_000);
        assert_eq!(opencoder_llm::estimate(&body), 20_000);
        let out = injectable_body(&body, "/s/SKILL.md");
        assert_eq!(out, body, "boundary is inclusive");
    }

    #[test]
    fn injectable_body_truncates_on_whole_lines_with_continuation_notice() {
        // 5 lines x 19,000 chars = 23,750 tokens total; 4 lines = 19,000 fit.
        let line = |n: usize| format!("L{n}-{}", "x".repeat(19_000 - 4));
        let body = (0..5).map(line).collect::<Vec<_>>().join("\n");
        assert!(opencoder_llm::estimate(&body) > 20_000);
        let out = injectable_body(&body, "/skills/big/SKILL.md");
        assert!(out.contains("[INCOMPLETE SKILL]"), "{:.80}", out);
        assert!(
            out.contains("1 lines remain; read the rest with the read tool: read(path=\"/skills/big/SKILL.md\", offset=5)."),
            "notice carries remaining count + 1-based next line: {}",
            &out[out.len().saturating_sub(200)..]
        );
        let prefix_end = out.find("\n[INCOMPLETE SKILL]").expect("notice after prefix");
        assert!(
            opencoder_llm::estimate(&out[..prefix_end]) <= 20_000,
            "prefix stays within budget"
        );
        assert!(
            out[..prefix_end].starts_with("L0-") && out[..prefix_end].contains("L3-"),
            "prefix keeps lines 0..=3"
        );
        assert!(!out[..prefix_end].contains("L4-"), "line 4 dropped");
    }

    #[test]
    fn injectable_body_single_oversized_line_degrades_to_notice_only() {
        // One line larger than the whole budget: zero lines fit; the notice
        // must still point at offset=1 so the model can chain reads.
        let body = "z".repeat(80_100);
        let out = injectable_body(&body, "/s/huge/SKILL.md");
        assert!(out.starts_with("[INCOMPLETE SKILL]"), "{out:.80}");
        assert!(
            out.contains("1 lines remain") && out.contains("offset=1"),
            "{out:.160}"
        );
    }

    #[test]
    fn full_body_marker_and_scan_semantics() {
        assert_eq!(full_body_marker("/a/SKILL.md"), "[skill loaded] /a/SKILL.md");
        let mk = |text: &str, synthetic: bool| {
            let mut m = Message::user("id", text);
            m.synthetic = synthetic;
            m
        };
        let hit = mk("[skill loaded] /a/SKILL.md\n\nbody", true);
        assert!(loaded_marker_matches(std::slice::from_ref(&hit), "/a/SKILL.md"));
        // Path-prefix collision guard: a marker for a LONGER path must not match.
        assert!(!loaded_marker_matches(&[hit], "/a/SKILL.md.bak"));
        // Non-synthetic or non-user messages never count.
        let plain = mk("[skill loaded] /a/SKILL.md\n\nbody", false);
        assert!(!loaded_marker_matches(&[plain], "/a/SKILL.md"));
        assert!(!loaded_marker_matches(&[], "/a/SKILL.md"));
    }

}
