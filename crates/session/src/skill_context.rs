//! Skill body delivery: ONE-SHOT per activation + tail reminder.
//!
//! Skill bodies do not ship in the system prompt — appending them there
//! would rewrite the payload's first bytes on every activation, destroying
//! whatever prefix-cache hits those bytes enjoyed. Instead the ACTIVE
//! skills' paths + merged body ship as ONE `synthetic` user message
//! ([`body_message`], a `[skill loaded] <path>` marker line per source
//! path — sorted, set-canonical) attached to the payload of the FIRST LLM
//! round that observes the armed skill, and ONLY that round: the delivery
//! gate ([`deliver_body_once`]) flips after the one shipment, so rounds
//! 2..N of the run carry NO skill body at all — no per-round token waste,
//! no cache-hostile duplicate block after the newest tool results. The
//! marker line names the source path, so a model that needs the skipped
//! body again just `read`s the SKILL.md (the same continuation the
//! `[INCOMPLETE SKILL]` notice already teaches).
//!
//! The message is never recorded to the transcript or store, and never
//! replayed — the in-memory delivery flag (`skill_body_delivered`, reset
//! by every `set_skill` and by the run-end clear) is the only ledger, so
//! compaction and resume need no marker-scan bookkeeping: a resume of a
//! crash-mid-run skill re-delivers once on its first round. That context
//! carries (a) the ACTIVE skills' paths + merged body so the model no
//! longer burns a tool call reading the SKILL.md, (b) a catalog of
//! config-enabled skills on the tail reminder (lazy-load hint for
//! `~/.opencoder/skills`) and (c) — only in the degenerate empty-body
//! case — the active skill's source path (from the `> Source:` prefix
//! `body_with_source` writes). Bodies over ~20K tokens ship as a
//! whole-line prefix plus an `[INCOMPLETE SKILL]` continuation notice
//! (same style as the read tool's `[INCOMPLETE READ]`).

use std::path::Path;

use opencoder_core::{discover_in, skills_dir, AgentMode, Config, Message};

use crate::SessionState;

/// Extract EVERY source-file path a skill prompt carries, in discovery
/// order (first occurrence wins on duplicates). A compound prompt
/// (`$A $B`) is stored by `skill_resolve` as
/// `> Source: <pathA>\n\n<bodyA>\n\n> Source: <pathB>\n\n<bodyB>` — every
/// paragraph-initial `> Source: <path>` line contributes one path; paths
/// may contain spaces because a path always runs to end-of-line.
pub fn source_paths_from_body(body: &str) -> Vec<&str> {
    let mut paths: Vec<&str> = Vec::new();
    let mut paragraph_start = true;
    for line in body.lines() {
        if line.trim().is_empty() {
            paragraph_start = true;
            continue;
        }
        if paragraph_start {
            if let Some(rest) = line.strip_prefix("> Source: ") {
                let path = rest.trim();
                if !path.is_empty() && !paths.contains(&path) {
                    paths.push(path);
                }
            }
        }
        paragraph_start = false;
    }
    paths
}

/// Extract the FIRST source-file path from a `> Source: <path>` prefix line
/// (written by `opencoder_core::body_with_source`). `None` when absent or
/// the path is empty. Single-skill convenience over
/// [`source_paths_from_body`].
pub fn source_path_from_body(body: &str) -> Option<&str> {
    source_paths_from_body(body).into_iter().next()
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

/// Enabled-skill catalog for the default skills directory. An unresolvable
/// home directory yields an empty catalog (skills cannot be listed — and were
/// never seeded — without one).
pub fn catalog_entries(config: &Config) -> Vec<(String, String)> {
    skills_dir()
        .map(|root| catalog_entries_in(&root, config))
        .unwrap_or_default()
}

/// Pure builder for the reminder text. Empty string when both inputs are
/// empty; otherwise the `[skills]` and/or `[active skill]` sections joined
/// by a blank line.
pub fn reminder_text(catalog: &[(String, String)], active_path: Option<&str>) -> String {
    let mut sections: Vec<String> = Vec::new();
    if !catalog.is_empty() {
        // Display path for the reminder: the real skills dir when resolvable,
        // else the canonical `~/.opencoder/skills` location (still actionable
        // even when the home directory could not be resolved).
        let root = skills_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_else(|| "~/.opencoder/skills".to_string());
        let mut lines = vec![format!("[skills]\nEnabled skills live under {root}:")];
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

/// One-read derivation of the transient skill payload for an armed skill:
/// `(full-body message, fallback pointer path)`. The body message exists
/// only while the skill is armed AND carries an injectable body; the
/// pointer is reserved for the degenerate armed-without-body case
/// (frontmatter-only file — nothing to ship, so pointing at the source file
/// is all the context there is). Shared by [`body_message`] and
/// [`tail_reminder`] so the two never disagree. Gating: Primary agents
/// only, `workflow` (the todos scheduler, itself Primary-mode) excluded.
fn body_and_pointer(session: &SessionState) -> (Option<Message>, Option<String>) {
    if session.agent.mode != AgentMode::Primary || session.agent.name == "workflow" {
        return (None, None);
    }
    let prompt = session.skill_prompt_cloned();
    let Some(prompt) = prompt.as_deref() else {
        return (None, None);
    };
    let paths = source_paths_from_body(prompt);
    if paths.is_empty() {
        // Legacy body without a `> Source:` prefix: no path to name in the
        // marker block and no path to point at — nothing to deliver.
        return (None, None);
    }
    // Strip the LEADING `> Source: <path>` prefix block: the marker block
    // already carries every path, and the injected lines must mirror the
    // file's own body so the truncation `offset=` matches what
    // `read(path, offset=…)` returns. A compound prompt keeps its inner
    // `> Source:` annotation, so bodyB ships together with bodyA.
    let body = prompt
        .strip_prefix("> Source: ")
        .and_then(|rest| rest.split_once("\n\n"))
        .map(|(_, body)| body)
        .unwrap_or(prompt);
    // A skill whose parsed body is empty (frontmatter-only file) carries
    // nothing beyond its source path: no message to append, and the tail
    // reminder keeps the fallback pointer.
    if body.trim().is_empty() {
        return (None, Some(paths[0].to_string()));
    }
    // The oversized-body continuation notice names the FIRST discovered
    // path (primary entry point of the compound body).
    let text = format!(
        "{}\n\n{}",
        full_body_marker_block(&paths),
        injectable_body(body, paths[0])
    );
    let mut msg = Message::user(crate::runner::new_id(), text);
    msg.synthetic = true;
    (Some(msg), None)
}

/// Build the per-call tail reminder message, or `None` when there is nothing
/// to remind about. Only Primary agents get skill context: subagents run
/// scoped tasks, and `workflow` — although itself a Primary-mode agent (see
/// `crates/core/src/agent.rs`) — is the todos scheduler and must not receive
/// it, hence the explicit name check.
///
/// The `[active skill]` section is a FALLBACK pointer, not a standing
/// announcement: while the armed skill's body is in the transcript
/// ([`body_message`], injected once by [`ensure_body_once`]), the pointer
/// would only make the model parrot "the <skill> skill is active" on every
/// turn, so it stays silent. It fires solely for an armed skill
/// whose parsed body is empty, where pointing at the source file is the
/// only context available.
pub fn tail_reminder(session: &SessionState) -> Option<Message> {
    if session.agent.mode != AgentMode::Primary || session.agent.name == "workflow" {
        return None;
    }
    let (_body, pointer) = body_and_pointer(session);
    let text = reminder_text(&catalog_entries(&session.config), pointer.as_deref());
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

/// Marker line heading the transient full-body message: `[skill loaded]
/// <path>`.
pub fn full_body_marker(path: &str) -> String {
    format!("[skill loaded] {path}")
}

/// Marker block heading the transient full-body message: one
/// `[skill loaded] <path>` line per source path, sorted lexicographically
/// and deduplicated — the canonical form, so `$A $B` and `$B $A` produce
/// the same block and the model sees one stable header.
pub fn full_body_marker_block(paths: &[&str]) -> String {
    let mut sorted: Vec<&str> = paths.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    sorted
        .iter()
        .map(|p| full_body_marker(p))
        .collect::<Vec<_>>()
        .join("\n")
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

/// Build the ONE-SHOT skill body message: the ACTIVE skills' paths + merged
/// body (marker block + blank line + [`injectable_body`] output), or `None`
/// when there is nothing to ship. Pure: derived from session state alone,
/// `synthetic=true`. The caller records it to the transcript exactly once
/// ([`ensure_body_once`]; runners carry it via the transcript from then on),
/// and run end (`skill_lifecycle::clear_on_run_end`) drops the skill, which
/// stops further submissions. A compound prompt (`$A $B`) ships as ONE
/// message keyed by the whole path set; bodies over ~20K tokens ship
/// truncated with a continuation notice.
pub fn body_message(session: &SessionState) -> Option<Message> {
    body_and_pointer(session).0
}

/// Deliver the armed skills' body EXACTLY ONCE per activation: the first
/// call after a skill arm returns [`body_message`] for the payload tail and
/// flips the session's delivery gate; every later round of that run (and
/// every later call) returns `None` — rounds 2..N carry NO skill body, the
/// `[skill loaded] <path>` marker on the delivered message being the
/// model's pointer back to the source file. Subagents and the `workflow`
/// scheduler never receive skill context (same gate as
/// [`body_and_pointer`]). The gate resets on every `set_skill` (new
/// activation -> new delivery) and at run end.
pub fn deliver_body_once(session: &SessionState) -> Option<Message> {
    if session.agent.mode != AgentMode::Primary || session.agent.name == "workflow" {
        return None;
    }
    if session.skill_body_delivered() {
        return None;
    }
    let msg = body_message(session)?;
    session.set_skill_body_delivered(true);
    Some(msg)
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

    fn act_session() -> SessionState {
        SessionState::new(
            "t",
            resolve_agent("act").unwrap(),
            Config::default(),
            client(),
            Path::new("/tmp").into(),
        )
    }

    fn session_named(name: &str) -> SessionState {
        SessionState::new(
            "t",
            resolve_agent(name).unwrap(),
            Config::default(),
            client(),
            Path::new("/tmp").into(),
        )
    }

    // ------------------------------------------------------------------
    // deliver_body_once: the one-shot gate
    // ------------------------------------------------------------------

    /// Exactly-once: the first call after arming returns the body message
    /// and flips the gate; every later call returns None until a fresh
    /// `set_skill` (new activation) resets the gate. Skill-less sessions
    /// and the excluded agents (subagents, `workflow`) never deliver.
    #[test]
    fn deliver_body_once_is_one_shot_per_activation() {
        let s = act_session();
        assert!(deliver_body_once(&s).is_none(), "skill-less -> None");

        s.set_skill(Some(sourced("/skills/rev/SKILL.md", "REV-BODY")));
        let msg = deliver_body_once(&s).expect("first call delivers");
        assert!(msg.text().starts_with("[skill loaded] /skills/rev/SKILL.md"));
        assert!(s.skill_body_delivered(), "gate flipped by the delivery");

        assert!(
            deliver_body_once(&s).is_none(),
            "second call: delivery already spent"
        );

        // A new activation re-arms the gate -> one fresh delivery.
        s.set_skill(Some(sourced("/skills/review/SKILL.md", "NEXT-BODY")));
        let msg2 = deliver_body_once(&s).expect("fresh activation delivers again");
        assert!(
            msg2.text().starts_with("[skill loaded] /skills/review/SKILL.md"),
            "the new body ships, not the stale one"
        );
        assert!(deliver_body_once(&s).is_none(), "spent again");

        for name in ["explore", "workflow"] {
            let s = session_named(name);
            s.set_skill(Some("> Source: /skills/x/SKILL.md\n\nX-BODY".into()));
            assert!(
                deliver_body_once(&s).is_none(),
                "{name} never receives the body"
            );
            assert!(!s.skill_body_delivered(), "{name}: gate untouched");
        }
    }

    // ------------------------------------------------------------------
    // body_message: gating
    // ------------------------------------------------------------------

    /// Skill-less sessions get no body message, and the exclusion set
    /// mirrors `tail_reminder`: subagents and the `workflow` scheduler are
    /// out even with a well-formed Source-prefixed skill armed.
    #[test]
    fn body_message_gating() {
        let s = act_session();
        assert!(body_message(&s).is_none(), "skill-less -> None");
        for name in ["explore", "workflow"] {
            let s = session_named(name);
            s.set_skill(Some("> Source: /skills/x/SKILL.md\n\nX-BODY".into()));
            assert!(
                body_message(&s).is_none(),
                "{name} never receives the body"
            );
        }
    }

    /// No `> Source:` prefix (legacy body) -> no marker path -> no message.
    /// An empty parsed body (frontmatter-only file) -> nothing to inject.
    #[test]
    fn body_message_none_for_legacy_or_empty_body() {
        let s = act_session();
        s.set_skill(Some("LEGACY-BODY-WITHOUT-PREFIX".into()));
        assert!(
            body_message(&s).is_none(),
            "legacy body has no path to name"
        );
        s.set_skill(Some("> Source: /skills/e/SKILL.md\n\n   \n".into()));
        assert!(
            body_message(&s).is_none(),
            "empty parsed body ships nothing"
        );
    }

    // ------------------------------------------------------------------
    // body_message: shape
    // ------------------------------------------------------------------

    /// Single skill: `[skill loaded] <path>` marker line, blank line, then
    /// the body verbatim (leading `> Source:` prefix block stripped).
    #[test]
    fn body_message_single_skill_shape() {
        let s = act_session();
        s.set_skill(Some("> Source: /skills/rev/SKILL.md\n\nREV-STEP-1\nREV-STEP-2".into()));
        let msg = body_message(&s).expect("armed with a body -> message");
        assert!(msg.synthetic, "transient, never recorded");
        assert_eq!(msg.role, Role::User);
        assert_eq!(
            msg.text(),
            "[skill loaded] /skills/rev/SKILL.md\n\nREV-STEP-1\nREV-STEP-2"
        );
    }

    /// Compound prompt (`$A $B`): ONE message whose sorted marker block
    /// carries every path and whose merged body keeps B's inner
    /// `> Source:` annotation. Set order is canonicalized (`$B $A` yields
    /// the same block), so re-arming with the same set in another order
    /// produces the identical payload.
    #[test]
    fn body_message_compound_shape_is_set_canonical() {
        let s = act_session();
        s.set_skill(Some(
            "> Source: /skills/alpha/SKILL.md\n\nALPHA-BODY\n\n\
             > Source: /skills/beta/SKILL.md\n\nBETA-BODY"
                .into(),
        ));
        let text = body_message(&s)
            .expect("compound body ships")
            .text();
        assert_eq!(
            text,
            "[skill loaded] /skills/alpha/SKILL.md\n\
             [skill loaded] /skills/beta/SKILL.md\n\n\
             ALPHA-BODY\n\n> Source: /skills/beta/SKILL.md\n\nBETA-BODY",
            "sorted block + merged body with inner annotation"
        );
        s.set_skill(Some(
            "> Source: /skills/beta/SKILL.md\n\nBETA-BODY\n\n\
             > Source: /skills/alpha/SKILL.md\n\nALPHA-BODY"
                .into(),
        ));
        let flipped = body_message(&s)
            .expect("same set, other order")
            .text();
        let block =
            "[skill loaded] /skills/alpha/SKILL.md\n[skill loaded] /skills/beta/SKILL.md\n\n";
        assert!(
            text.starts_with(block) && flipped.starts_with(block),
            "marker block is canonical: order-insensitive"
        );
        // The merged body follows the prompt's discovery order, each body
        // adjacent to its own inner `> Source:` annotation.
        assert!(
            flipped.contains("BETA-BODY\n\n> Source: /skills/alpha/SKILL.md\n\nALPHA-BODY"),
            "flipped order keeps bodies with their annotations: {flipped}"
        );
    }

    /// Oversized body (>20K est tokens): the message ships the largest
    /// whole-line prefix within budget plus the `[INCOMPLETE SKILL]`
    /// notice whose `offset=` matches the read tool convention; the
    /// dropped lines never enter the message.
    #[test]
    fn body_message_oversized_truncates_with_continuation_notice() {
        // 5 lines x ~19K chars ~= 23.7K tokens; 4 lines ~= 19K fit.
        let line = |n: usize| format!("BIG-{n:02} {}", "x".repeat(19_000));
        let body = (0..5usize).map(line).collect::<Vec<_>>().join("\n");
        assert!(opencoder_llm::estimate(&body) > 20_000);
        let s = act_session();
        s.set_skill(Some(sourced("/skills/big/SKILL.md", &body)));

        let text = body_message(&s)
            .expect("truncation still ships a message")
            .text();
        assert!(
            text.starts_with("[skill loaded] /skills/big/SKILL.md\n\n"),
            "marker block still leads: {:.80}",
            text
        );
        assert!(
            text.contains(
                "[INCOMPLETE SKILL] truncated at ~20K tokens; 1 lines remain; \
                 read the rest with the read tool: read(path=\"/skills/big/SKILL.md\", offset=5).",
            ),
            "notice names remaining lines + 1-based next offset: {}",
            &text[text.len().saturating_sub(220)..]
        );
        let cut = text
            .find("\n[INCOMPLETE SKILL]")
            .expect("notice follows the prefix");
        assert!(
            opencoder_llm::estimate(&text[..cut]) <= 20_000,
            "marker + truncated prefix stays within budget"
        );
        assert!(text[..cut].contains("BIG-03"), "lines 0..=3 kept");
        assert!(!text[..cut].contains("BIG-04"), "line 4 dropped");
    }

    // ------------------------------------------------------------------
    // tail_reminder: pointer reserved for the bodyless skill
    // ------------------------------------------------------------------

    /// The `[active skill]` tail is a FALLBACK pointer, not a standing
    /// announcement: while the armed skill's body has shipped (one-shot,
    /// first round after activation), the pointer stays silent — repeating
    /// it every turn only makes the model parrot "the <skill> skill is
    /// active". It fires solely for an armed skill whose
    /// parsed body is empty, where the source path is all the context
    /// there is. It never depends on transcript contents, so compaction
    /// and resume need no re-pointing: the armed rounds re-derive
    /// everything.
    #[test]
    fn tail_reminder_pointer_reserved_for_bodyless_skill() {
        let s = act_session();
        s.set_skill(Some("> Source: /skills/rev/SKILL.md\n\nREV".into()));
        assert!(
            tail_reminder(&s).is_none(),
            "armed with a body -> pointer suppressed (body ships adjacent)"
        );

        // Degenerate: frontmatter-only skill (empty parsed body).
        s.set_skill(Some("> Source: /skills/bare/SKILL.md\n\n".into()));
        let tail = tail_reminder(&s).expect("bodyless skill keeps the pointer");
        let text = tail.text();
        assert!(
            text.contains("[active skill]") && text.contains("/skills/bare/SKILL.md"),
            "pointer names the source path: {text}"
        );

        // Skill-less again: nothing to remind about.
        s.set_skill(None);
        assert!(tail_reminder(&s).is_none(), "skill-less -> no tail");
    }

    /// Gating and shape of the tail itself: Primary agents only, `workflow`
    /// excluded, message is a synthetic user turn (never recorded).
    #[test]
    fn tail_reminder_gating_and_content() {
        let s = act_session();
        // Bodyless skill -> the pointer fires as a synthetic user message.
        s.set_skill(Some("> Source: /skills/bare/SKILL.md\n\n".into()));
        let msg = tail_reminder(&s).expect("primary + Source prefix");
        assert_eq!(msg.role, Role::User);
        assert!(msg.synthetic, "transient, never recorded");
        assert!(msg.text().contains("[active skill]"));
        assert!(
            tail_reminder(&session_named("workflow")).is_none(),
            "workflow excluded"
        );
        assert!(
            tail_reminder(&session_named("explore")).is_none(),
            "subagent excluded"
        );
    }

    // ------------------------------------------------------------------
    // kept pure helpers
    // ------------------------------------------------------------------

    fn sourced(path: &str, body: &str) -> String {
        format!("> Source: {path}\n\n{body}")
    }

    #[test]
    fn full_body_marker_and_block_are_canonical() {
        assert_eq!(
            full_body_marker("/a/SKILL.md"),
            "[skill loaded] /a/SKILL.md"
        );
        // Marker block: one line per path, sorted + deduped (canonical).
        assert_eq!(
            full_body_marker_block(&["/b/SKILL.md", "/a/SKILL.md", "/b/SKILL.md"]),
            "[skill loaded] /a/SKILL.md\n[skill loaded] /b/SKILL.md"
        );
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
        let body: String = (0..5usize)
            .map(|i| format!("L{i}-{}", "x".repeat(19_000)))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(opencoder_llm::estimate(&body) > 20_000);
        let out = injectable_body(&body, "/skills/big/SKILL.md");
        assert!(out.contains("[INCOMPLETE SKILL]"), "{:.80}", out);
        assert!(
            out.contains("1 lines remain; read the rest with the read tool: read(path=\"/skills/big/SKILL.md\", offset=5)."),
            "notice carries remaining count + 1-based next line: {}",
            &out[out.len().saturating_sub(200)..]
        );
        let prefix_end = out
            .find("\n[INCOMPLETE SKILL]")
            .expect("notice after prefix");
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
    fn source_paths_from_body_variants() {
        // Compound prompt as `skill_resolve` stores it: discovery order,
        // first occurrence wins on duplicates.
        let compound = "> Source: /s/alpha/SKILL.md\n\nA\n\n> Source: /s/beta/SKILL.md\n\nB";
        assert_eq!(
            source_paths_from_body(compound),
            vec!["/s/alpha/SKILL.md", "/s/beta/SKILL.md"]
        );
        // Only PARAGRAPH-initial lines count: a mid-paragraph mention is
        // body text, not a source marker.
        assert_eq!(
            source_paths_from_body("> Source: /s/a/SKILL.md\n\nkeep\n> Source: mid"),
            vec!["/s/a/SKILL.md"]
        );
    }
}
