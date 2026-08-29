//! Terminal rendering of [`SessionEvent`]s for headless output.

use opencoder_session::SessionEvent;

/// Print a session event to stdout/stderr with ANSI colours.
pub(crate) fn print_event(ev: &SessionEvent) {
    match ev {
        SessionEvent::LlmRoundStart { .. }
        | SessionEvent::LlmRoundEnd
        | SessionEvent::LlmUsage { .. } => {}
        SessionEvent::TextDelta(t) => {
            print!("{t}");
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
        SessionEvent::ReasoningDelta(_) => {}
        SessionEvent::CompactionDelta(t) => {
            // Stream the summary live (muted/dim) so it appears while the
            // summarizing LLM call runs, not only after the final Compaction.
            eprint!("[2m{t}[0m");
            use std::io::Write;
            let _ = std::io::stderr().flush();
        }
        SessionEvent::ToolStart { name, input, .. } => {
            if name == "task" {
                return;
            }
            eprintln!(
                "\n\x1b[36m\u{25b8} {name}\x1b[0m {}",
                summarize_input(input)
            );
        }
        SessionEvent::ToolEnd {
            name,
            output,
            is_error,
            ..
        } => {
            let color = if *is_error { "31" } else { "2" };
            eprintln!("\x1b[{color}m  {}\x1b[0m", indent_first(output, 2));
            let _ = name;
        }
        SessionEvent::AgentSwitch(to) => {
            eprintln!("\n\x1b[35m[switched to {to} mode]\x1b[0m");
        }
        SessionEvent::ModelSwitch(to) => {
            eprintln!("\n\x1b[35m[switched to model: {to}]\x1b[0m");
        }
        SessionEvent::Compaction(s) => {
            eprintln!("\n\x1b[33m[context compacted]\x1b[0m {}", truncate(s, 160));
        }
        SessionEvent::Status(s) => {
            eprintln!("\x1b[2m[{s}]\x1b[0m");
        }
        SessionEvent::Done => {
            println!("\n");
        }
        SessionEvent::Error(e) => {
            eprintln!("\n\x1b[31merror: {e}\x1b[0m");
        }
        SessionEvent::SubagentStart { kind, prompt, .. } => {
            eprintln!("\x1b[34m\u{2937} subagent [{kind}] {prompt}\x1b[0m");
        }
        SessionEvent::SubagentEnd { ok, summary, .. } => {
            let mark = if *ok { "\u{2714}" } else { "\u{2718}" };
            eprintln!("\x1b[34m  {mark} {summary}\x1b[0m");
        }
        SessionEvent::TranscriptReset(cleared) => {
            eprintln!("{}", transcript_reset_banner(cleared.len()));
        }
        SessionEvent::QueueConsumed { text, .. } => {
            if !text.is_empty() {
                println!("user: {text}");
            }
        }
        SessionEvent::SteerConsumed { text, .. } => {
            if !text.is_empty() {
                println!("user: {text}");
            }
        }
        SessionEvent::SubagentChild { .. } => {}
        SessionEvent::AutoPilot { phase, iteration } => {
            eprintln!("\n\x1b[35m\u{25c9} autopilot: {phase:?} (iteration {iteration})\x1b[0m");
        }
    }
}

/// Build a short one-line summary of a tool-call's JSON `input`.
pub(crate) fn summarize_input(input: &serde_json::Value) -> String {
    match input {
        serde_json::Value::Object(map) => {
            if let Some(c) = map.get("command").and_then(|v| v.as_str()) {
                return truncate(c, 100);
            }
            if let Some(c) = map.get("path").and_then(|v| v.as_str()) {
                return truncate(c, 100);
            }
            if let Some(c) = map.get("description").and_then(|v| v.as_str()) {
                return truncate(c, 100);
            }
            let s = serde_json::to_string(input).unwrap_or_default();
            truncate(&s, 100)
        }
        other => {
            let s = serde_json::to_string(other).unwrap_or_default();
            truncate(&s, 100)
        }
    }
}

/// Fold-boundary banner for `/act_clear_context` (the session emits
/// [`SessionEvent::TranscriptReset`] with the cleared transcript). Replaces
/// the old plan-handoff banner: a clear-context fold must stay visible in
/// headless output, otherwise the model's next reply appears to answer
/// nothing. Pure (count -> text) so the wording is unit-testable without
/// capturing stderr.
pub(crate) fn transcript_reset_banner(cleared: usize) -> String {
    let plural = if cleared == 1 { "" } else { "s" };
    format!(
        "\n\x1b[33m\u{2500}\u{2500} context cleared ({cleared} message{plural} folded) \u{2500}\u{2500}\x1b[0m"
    )
}

/// Truncate a string to at most `n` characters, appending `...` when trimmed.
pub(crate) fn truncate(s: &str, n: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= n {
        t.to_string()
    } else {
        let cut: String = t.chars().take(n).collect();
        format!("{cut}...")
    }
}

/// Indent every line of `s` by `n` spaces.
pub(crate) fn indent_first(s: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    s.lines()
        .map(|l| format!("{pad}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_input_extracts_command() {
        let input = serde_json::json!({"command": "ls -la"});
        assert_eq!(summarize_input(&input), "ls -la");
    }

    #[test]
    fn summarize_input_extracts_path_when_no_command() {
        let input = serde_json::json!({"path": "/tmp/foo.rs"});
        assert_eq!(summarize_input(&input), "/tmp/foo.rs");
    }

    #[test]
    fn truncate_adds_ellipsis() {
        let long = "a".repeat(120);
        let t = truncate(&long, 10);
        assert!(t.ends_with("..."));
        assert_eq!(t.chars().count(), 13); // 10 + "..."
    }

    #[test]
    fn truncate_short_returns_as_is() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn indent_first_pads_each_line() {
        let s = "line1\nline2";
        assert_eq!(indent_first(s, 2), "  line1\n  line2");
    }

    #[test]
    fn transcript_reset_banner_counts_folded_messages() {
        // Singular and plural stay grammatical; the banner is a visible
        // fold-boundary marker (replaces the removed plan-handoff banner).
        assert!(transcript_reset_banner(1).contains("1 message folded"));
        assert!(transcript_reset_banner(3).contains("3 messages folded"));
        let banner = transcript_reset_banner(0);
        assert!(banner.contains("context cleared"), "banner must mark the fold: {banner}");
    }
}
