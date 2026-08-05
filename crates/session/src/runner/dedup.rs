use opencoder_core::ToolOutput;
use serde_json::Value;

/// Deduplicate consecutive bash-timeout tool results.
///
/// When bash times out repeatedly (e.g. the model keeps running long commands
/// across turns), only the first timeout's full message is kept \u{2014} subsequent
/// ones are replaced with the first's content (same PID, same output file
/// path, so the model reads the same file regardless). A non-timeout bash
/// result resets the consecutive count. Non-bash tool calls do NOT reset it
/// (they run independently and don't affect the bash timeout streak).
///
/// Deduplication only collapses timeouts for the SAME command: a different
/// command has its own PID and output-file path, so reusing the first
/// timeout's content would hide the real PID and point the model at the wrong
/// background output file. A mismatched command starts a new streak.
///
/// `first` persists across turns (it lives in `run_loop`'s scope) so the
/// dedup applies across turn boundaries, not just within a single batch.
pub(super) fn dedup_consecutive_bash_timeouts(
    tool_calls: &[opencoder_llm::CompletedToolCall],
    results: &mut [(usize, ToolOutput)],
    first: &mut Option<(String, Value)>,
) {
    for (i, out) in results.iter_mut() {
        let tc = tool_calls.get(*i);
        let is_bash = tc.is_some_and(|tc| tc.name == "bash");
        if is_bash
            && out
                .content
                .starts_with(crate::tools::bash::BASH_TIMEOUT_MARKER)
        {
            // Capture the command input so timeouts for different commands
            // are NOT collapsed onto each other.
            let input = tc.map(|tc| tc.input.clone()).unwrap_or(Value::Null);
            if let Some((first_content, first_input)) = first {
                if *first_input == input {
                    out.content = first_content.clone();
                } else {
                    // Different command — start a fresh streak so this
                    // timeout's own PID / output file is preserved.
                    *first = Some((out.content.clone(), input));
                }
            } else {
                *first = Some((out.content.clone(), input));
            }
        } else if is_bash {
            *first = None;
        }
    }
}

#[cfg(test)]
mod dedup_tests {
    use super::dedup_consecutive_bash_timeouts;
    use opencoder_core::ToolOutput;
    use opencoder_llm::CompletedToolCall;
    use serde_json::json;

    fn bash_tc(id: &str, command: &str) -> CompletedToolCall {
        CompletedToolCall {
            id: id.into(),
            name: "bash".into(),
            input: json!({ "command": command }),
        }
    }
    fn other_tc(name: &str, id: &str) -> CompletedToolCall {
        CompletedToolCall {
            id: id.into(),
            name: name.into(),
            input: json!({}),
        }
    }
    fn timeout_output(pid: u32) -> ToolOutput {
        ToolOutput {
            content: format!(
                "[bash-timeout: command timed out after 1s \u{2014} moved to background]\n\
                 pid: {pid}\noutput: /tmp/opencoder_bg_{pid}.output\n\n"
            ),
            is_error: false,
            images: vec![],
        }
    }
    fn normal_output(text: &str) -> ToolOutput {
        ToolOutput::ok(text)
    }

    #[test]
    fn first_timeout_stored_subsequent_replaced() {
        let tool_calls = vec![bash_tc("1", "sleep 10"), bash_tc("2", "sleep 10")];
        let mut results = vec![(0, timeout_output(100)), (1, timeout_output(200))];
        let mut first = None;
        dedup_consecutive_bash_timeouts(&tool_calls, &mut results, &mut first);
        assert_eq!(
            results[0].1.content, results[1].1.content,
            "second timeout content must match first"
        );
        assert!(results[0].1.content.contains("pid: 100"));
        assert!(
            results[1].1.content.contains("pid: 100"),
            "second must be replaced with first content (pid 100)"
        );
    }

    #[test]
    fn non_timeout_bash_resets_count() {
        let tool_calls = vec![
            bash_tc("1", "sleep 10"),
            bash_tc("2", "ls"),
            bash_tc("3", "sleep 10"),
        ];
        let mut results = vec![
            (0, timeout_output(100)),
            (1, normal_output("done")),
            (2, timeout_output(300)),
        ];
        let mut first = None;
        dedup_consecutive_bash_timeouts(&tool_calls, &mut results, &mut first);
        assert!(results[0].1.content.contains("pid: 100"));
        assert!(results[1].1.content.contains("done"));
        assert!(
            results[2].1.content.contains("pid: 300"),
            "third timeout must have own content after reset"
        );
    }

    #[test]
    fn non_bash_tool_does_not_reset_count() {
        let tool_calls = vec![
            bash_tc("1", "sleep 10"),
            other_tc("edit", "2"),
            bash_tc("3", "sleep 10"),
        ];
        let mut results = vec![
            (0, timeout_output(100)),
            (1, normal_output("edited")),
            (2, timeout_output(300)),
        ];
        let mut first = None;
        dedup_consecutive_bash_timeouts(&tool_calls, &mut results, &mut first);
        assert!(results[0].1.content.contains("pid: 100"));
        assert!(results[1].1.content.contains("edited"));
        assert!(
            results[2].1.content.contains("pid: 100"),
            "third timeout must reuse first content (non-bash didn't reset)"
        );
    }

    #[test]
    fn first_persists_across_batches() {
        let tool_calls_a = vec![bash_tc("1", "sleep 10")];
        let mut results_a = vec![(0, timeout_output(100))];
        let mut first = None;
        dedup_consecutive_bash_timeouts(&tool_calls_a, &mut results_a, &mut first);

        let tool_calls_b = vec![bash_tc("2", "sleep 10")];
        let mut results_b = vec![(0, timeout_output(200))];
        dedup_consecutive_bash_timeouts(&tool_calls_b, &mut results_b, &mut first);

        assert!(
            results_b[0].1.content.contains("pid: 100"),
            "second-batch timeout must reuse first-batch content"
        );
    }

    #[test]
    fn different_commands_not_deduped() {
        // Two consecutive bash timeouts for DIFFERENT commands must NOT be
        // collapsed onto each other: each has its own PID / output file, and
        // reusing the first's content would hide the real PID and point the
        // model at the wrong background output file.
        let tool_calls = vec![bash_tc("1", "cargo build"), bash_tc("2", "npm test")];
        let mut results = vec![(0, timeout_output(100)), (1, timeout_output(200))];
        let mut first = None;
        dedup_consecutive_bash_timeouts(&tool_calls, &mut results, &mut first);
        assert!(
            results[0].1.content.contains("pid: 100"),
            "first timeout keeps its own pid"
        );
        assert!(
            results[1].1.content.contains("pid: 200"),
            "different-command timeout must keep its own pid (not deduped)"
        );
        assert!(
            !results[1].1.content.contains("pid: 100"),
            "different-command timeout must not inherit the first's pid"
        );
    }

    #[test]
    fn command_mismatch_starts_new_streak() {
        // timeout(A) -> timeout(B) -> timeout(A): the third (A) should dedup
        // against the SECOND (B), not the first (A), because B started a new
        // streak. Verifies the streak state updates on a mismatch.
        let tool_calls = vec![
            bash_tc("1", "cargo build"),
            bash_tc("2", "npm test"),
            bash_tc("3", "npm test"),
        ];
        let mut results = vec![
            (0, timeout_output(100)),
            (1, timeout_output(200)),
            (2, timeout_output(300)),
        ];
        let mut first = None;
        dedup_consecutive_bash_timeouts(&tool_calls, &mut results, &mut first);
        assert!(results[0].1.content.contains("pid: 100"));
        assert!(
            results[1].1.content.contains("pid: 200"),
            "mismatched command keeps its own pid (new streak)"
        );
        assert!(
            results[2].1.content.contains("pid: 200"),
            "third dedups against the second (same command, same streak)"
        );
    }
}
