//! Transient skill-context tail reminder.
//!
//! Skill bodies do not ship in the system prompt — appending them there
//! would rewrite the payload's first bytes on every activation, destroying
//! whatever prefix-cache hits those bytes enjoyed. Instead every LLM call
//! derives, from session state alone, one synthetic user message appended
//! at the END of the request payload: never recorded to the store, never
//! replayed. (Note: the system prompt as a whole is re-derived per call and
//! re-reads AGENTS.md from disk, so it is not byte-stable across calls; the
//! tail design only keeps skill activation from rewriting it.) The tail
//! carries (a) a catalog of config-enabled skills (lazy-load hint for
//! `~/.opencoder/skills`) and (b) the active skill's source path (from the
//! `> Source:` prefix `body_with_source` writes).
//!
//! On top of the transient tail, [`ensure_full_body_loaded`] (called by
//! `run_loop` once per LLM round, after compaction) idempotently injects the
//! ACTIVE skills' paths + merged body as ONE persistent `synthetic=true`
//! user message (a `[skill loaded] <path>` marker line per source path —
//! sorted, set-exact), so the model no longer burns a tool call reading the
//! SKILL.md. A compound prompt (`$A $B`) is keyed by the WHOLE path set:
//! re-activating with a different set (add or drop a skill) re-injects,
//! because a stale single-path marker no longer matches. Bodies over ~20K
//! tokens are injected as a whole-line prefix plus an `[INCOMPLETE SKILL]`
//! continuation notice (same style as the read tool's `[INCOMPLETE READ]`);
//! the tail reminder then only points back at that message as a fallback
//! (e.g. after compaction folded it into a summary, which triggers a fresh
//! injection).

use std::collections::BTreeSet;
use std::path::Path;

use opencoder_core::{discover_in, skills_dir, AgentMode, Config, Message, Role};

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
    // The `[active skill]` section is a FALLBACK pointer, not a standing
    // announcement: `ensure_full_body_loaded` runs before every LLM round, so
    // whenever the transcript already carries the `[skill loaded]` body
    // message covering the same source-path set, repeating the pointer every
    // round only makes the model parrot "the <skill> skill is active" on
    // every turn. Drop the section while that marker is present; it returns
    // automatically once compaction folds the marker message away.
    let paths = skill_prompt.as_deref().map(source_paths_from_body);
    let loaded = paths
        .as_deref()
        .is_some_and(|ps| loaded_marker_matches(&session.messages, ps));
    let active_path = if loaded {
        None
    } else {
        skill_prompt.as_deref().and_then(source_path_from_body)
    };
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

/// Marker block heading the persistent full-body message: one
/// `[skill loaded] <path>` line per source path, sorted lexicographically
/// and deduplicated — the canonical form, so `$A $B` and `$B $A` share one
/// block and matching ([`loaded_marker_matches`]) is plain set equality.
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

/// Leading `[skill loaded] <path>` marker lines of a message text. Empty
/// when the text does not begin with a marker; a marker line only counts
/// when newline-terminated (so `/a/SKILL.md` can never be carved out of a
/// marker written for `/a/SKILL.md.bak`).
fn marker_paths(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut rest = text;
    while let Some(after) = rest.strip_prefix("[skill loaded] ") {
        let Some((path, tail)) = after.split_once('\n') else {
            break;
        };
        let path = path.trim();
        if path.is_empty() {
            break;
        }
        paths.push(path.to_string());
        rest = tail;
    }
    paths
}

/// Whether the transcript already carries a `[skill loaded]` message whose
/// leading marker block covers EXACTLY `paths` — the idempotence gate for
/// [`ensure_full_body_loaded`] (same marker-scan precedent as the
/// compaction summary). Set equality: a marker covering a subset or
/// superset does NOT match, so any change in the active skill set (adding
/// `$B` to `$A`, or dropping `$B` again) triggers a fresh injection. Each
/// marker line must be newline-terminated after its path, keeping the
/// `/a/SKILL.md` vs `/a/SKILL.md.bak` prefix-collision guard.
pub fn loaded_marker_matches(messages: &[Message], paths: &[&str]) -> bool {
    let expected: BTreeSet<&str> = paths
        .iter()
        .copied()
        .filter(|p| !p.trim().is_empty())
        .collect();
    if expected.is_empty() {
        return false;
    }
    messages.iter().any(|m| {
        m.synthetic && m.role == Role::User && {
            let parsed = marker_paths(&m.text());
            let got: BTreeSet<&str> = parsed.iter().map(String::as_str).collect();
            got == expected
        }
    })
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
    let paths = source_paths_from_body(prompt);
    if paths.is_empty() {
        return;
    };
    if loaded_marker_matches(&session.messages, &paths) {
        return;
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
    // nothing beyond the path the transient tail already points at;
    // injecting would record a marker-only message.
    if body.trim().is_empty() {
        return;
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

    fn act_session() -> SessionState {
        SessionState::new(
            "t",
            resolve_agent("act").unwrap(),
            Config::default(),
            client(),
            Path::new("/tmp").into(),
        )
    }

    #[tokio::test]
    async fn compound_set_growth_reinjectects_merged_body() {
        let mut s = act_session();
        s.set_skill(Some(
            "> Source: /skills/alpha/SKILL.md\n\nALPHA-BODY".into(),
        ));
        ensure_full_body_loaded(&mut s).await;
        assert_eq!(s.messages.len(), 1, "first activation injects once");
        // `$A` -> `$A $B`: the active set changed, so the merged body must
        // be re-injected — B's body has to enter the context even though
        // A's old marker is already on record.
        s.set_skill(Some(
            "> Source: /skills/alpha/SKILL.md\n\nALPHA-BODY\n\n\
             > Source: /skills/beta/SKILL.md\n\nBETA-BODY"
                .into(),
        ));
        ensure_full_body_loaded(&mut s).await;
        assert_eq!(
            s.messages.len(),
            2,
            "set change must trigger a fresh injection"
        );
        // Content completeness: the fresh message carries BOTH marker
        // lines (canonical sorted block) and both body sections.
        let text = s.messages[1].text();
        assert!(s.messages[1].synthetic && s.messages[1].role == Role::User);
        assert!(
            text.starts_with(
                "[skill loaded] /skills/alpha/SKILL.md\n\
                 [skill loaded] /skills/beta/SKILL.md\n\n"
            ),
            "marker block leads the message: {text}"
        );
        assert!(
            text.contains("ALPHA-BODY") && text.contains("BETA-BODY"),
            "merged body ships whole: {text}"
        );
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

    /// Regression for the "every turn re-announces the active skill" bug:
    /// the `[active skill]` tail is a FALLBACK pointer. Once
    /// `ensure_full_body_loaded` has recorded the matching `[skill loaded]`
    /// marker, the pointer must stay silent; it returns only when the
    /// marker leaves the transcript (compaction) or a different path set
    /// becomes active.
    #[tokio::test]
    async fn tail_reminder_is_fallback_only_while_loaded_marker_present() {
        let mut s = act_session();
        s.set_skill(Some("> Source: /skills/rev/SKILL.md\n\nREV".into()));

        // First round: no marker on record yet -> the fallback pointer
        // fires (that is how the model learns where the body lives).
        assert!(tail_reminder(&s).is_some(), "no marker yet -> pointer fires");

        // The real load path records the marker as a synthetic message;
        // from the next round on the pointer must be suppressed.
        ensure_full_body_loaded(&mut s).await;
        assert_eq!(s.messages.len(), 1, "body injected exactly once");
        assert!(
            tail_reminder(&s).is_none(),
            "matching [skill loaded] marker suppresses the [active skill] tail"
        );

        // Compaction folds the marker away: the pointer returns so the
        // model can re-read the source file.
        s.messages.clear();
        let tail = tail_reminder(&s).expect("marker gone -> pointer returns");
        let text = tail.text();
        assert!(text.contains("[active skill]") && text.contains("/skills/rev/SKILL.md"));

        // A marker covering a DIFFERENT path set does not silence it.
        s.messages.push({
            let mut m = Message::user("id", "[skill loaded] /other/SKILL.md\n\nbody");
            m.synthetic = true;
            m
        });
        assert!(
            tail_reminder(&s).is_some(),
            "non-matching marker keeps the fallback pointer"
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
    fn full_body_marker_and_scan_semantics() {
        assert_eq!(
            full_body_marker("/a/SKILL.md"),
            "[skill loaded] /a/SKILL.md"
        );
        // Marker block: one line per path, sorted + deduped (canonical).
        assert_eq!(
            full_body_marker_block(&["/b/SKILL.md", "/a/SKILL.md", "/b/SKILL.md"]),
            "[skill loaded] /a/SKILL.md\n[skill loaded] /b/SKILL.md"
        );
        let mk = |text: &str, synthetic: bool| {
            let mut m = Message::user("id", text);
            m.synthetic = synthetic;
            m
        };
        let hit = mk("[skill loaded] /a/SKILL.md\n\nbody", true);
        assert!(loaded_marker_matches(
            std::slice::from_ref(&hit),
            &["/a/SKILL.md"]
        ));
        // Path-prefix collision guard: a marker for a LONGER path must not
        // match (whole-line parsing + set equality).
        assert!(!loaded_marker_matches(
            std::slice::from_ref(&hit),
            &["/a/SKILL.md.bak"]
        ));
        // Set-exact gate: subset and superset both fail -> re-inject.
        let both = mk(
            "[skill loaded] /a/SKILL.md\n[skill loaded] /b/SKILL.md\n\nbody",
            true,
        );
        let one = std::slice::from_ref(&hit);
        let two = std::slice::from_ref(&both);
        assert!(loaded_marker_matches(two, &["/a/SKILL.md", "/b/SKILL.md"]));
        // Order-insensitive: marker block is canonicalized by sorting.
        assert!(loaded_marker_matches(two, &["/b/SKILL.md", "/a/SKILL.md"]));
        assert!(!loaded_marker_matches(two, &["/a/SKILL.md"]), "subset");
        assert!(
            !loaded_marker_matches(one, &["/a/SKILL.md", "/b/SKILL.md"]),
            "superset"
        );
        // An unterminated final marker line never counts.
        let bare = mk("[skill loaded] /a/SKILL.md", true);
        assert!(!loaded_marker_matches(
            std::slice::from_ref(&bare),
            &["/a/SKILL.md"]
        ));
        // Non-synthetic or non-user messages never count; neither does an
        // empty path set.
        let plain = mk("[skill loaded] /a/SKILL.md\n\nbody", false);
        assert!(!loaded_marker_matches(
            std::slice::from_ref(&plain),
            &["/a/SKILL.md"]
        ));
        assert!(!loaded_marker_matches(&[], &["/a/SKILL.md"]));
        assert!(!loaded_marker_matches(one, &[]), "empty set never matches");
        assert!(!loaded_marker_matches(one, &[""]), "blank path ignored");
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
        assert_eq!(
            source_paths_from_body("> Source: /s/a x/SKILL.md\n\nbody"),
            vec!["/s/a x/SKILL.md"],
            "paths may contain spaces (whole-line parse)"
        );
        assert_eq!(
            source_paths_from_body("> Source: /s/a/SKILL.md\n\nA\n\n> Source: /s/a/SKILL.md\n\nA2"),
            vec!["/s/a/SKILL.md"],
            "duplicate path deduped"
        );
        assert!(source_paths_from_body("no prefix").is_empty());
        assert!(source_paths_from_body("> Source: \n\nb").is_empty());
        // Only paragraph-initial annotations count: a `> Source:` line in
        // the middle of a body paragraph is body text, not a source marker.
        assert_eq!(
            source_paths_from_body("> Source: /s/a/SKILL.md\n\nkeep\n> Source: mid"),
            vec!["/s/a/SKILL.md"]
        );
    }

    #[tokio::test]
    async fn compound_same_set_is_idempotent() {
        let mut s = act_session();
        s.set_skill(Some(
            "> Source: /skills/alpha/SKILL.md\n\nALPHA-BODY\n\n\
             > Source: /skills/beta/SKILL.md\n\nBETA-BODY"
                .into(),
        ));
        ensure_full_body_loaded(&mut s).await;
        assert_eq!(s.messages.len(), 1);
        // Same set (even written in the opposite order) -> no re-injection.
        s.set_skill(Some(
            "> Source: /skills/beta/SKILL.md\n\nBETA-BODY\n\n\
             > Source: /skills/alpha/SKILL.md\n\nALPHA-BODY"
                .into(),
        ));
        ensure_full_body_loaded(&mut s).await;
        assert_eq!(s.messages.len(), 1, "same set is one-shot");
    }

    #[tokio::test]
    async fn compound_set_shrink_reinjectects_single_body() {
        let mut s = act_session();
        s.set_skill(Some(
            "> Source: /skills/alpha/SKILL.md\n\nALPHA-BODY\n\n\
             > Source: /skills/beta/SKILL.md\n\nBETA-BODY"
                .into(),
        ));
        ensure_full_body_loaded(&mut s).await;
        assert_eq!(s.messages.len(), 1);
        // `$A $B` -> `$A`: the set shrank; the merged-marker message no
        // longer matches, so a fresh single-path injection must land.
        s.set_skill(Some(
            "> Source: /skills/alpha/SKILL.md\n\nALPHA-BODY".into(),
        ));
        ensure_full_body_loaded(&mut s).await;
        assert_eq!(s.messages.len(), 2, "shrink re-injects");
        let text = s.messages[1].text();
        assert!(
            text.starts_with("[skill loaded] /skills/alpha/SKILL.md\n\nALPHA-BODY"),
            "single-path marker + body: {text}"
        );
        assert!(
            !text.contains("BETA-BODY"),
            "shrunk injection carries only A: {text}"
        );
    }
}
