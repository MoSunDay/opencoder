//! 合并对头部与 subagent 派发场景的回归：turn 最后一个 step 要启动
//! subagent 时，`Say(n steps): <preview>` 的 preview 必须是 markdown
//! 渲染后的首行（头部不露 `#`/`**`/`-` 原始标记），且正文不把首行
//! 换个形态再复述一遍——两个症状（头部 markdown 不渲染 / 头部下方
//! 复述正文）同一根因：preview 与去重口径不一致。

use super::super::*;

fn lines(v: &ChatView) -> Vec<String> {
    v.flatten()
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<String>()
        })
        .collect()
}

fn bash(v: &mut ChatView, id: &str) {
    v.apply(&SessionEvent::ToolStart {
        id: id.into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: id.into(),
        name: "bash".into(),
        output: "out".into(),
        is_error: false,
        images: Vec::new(),
    });
}

fn task_start(v: &mut ChatView, id: &str) {
    v.apply(&SessionEvent::ToolStart {
        id: id.into(),
        name: "task".into(),
        input: serde_json::json!({"prompt": "p", "kind": "explore"}),
    });
}

fn subagent_start(v: &mut ChatView) {
    v.apply(&SessionEvent::SubagentStart {
        id: "s1".into(),
        kind: "explore".into(),
        prompt: "p".into(),
        child_session_id: "c1".into(),
    });
}

/// 用户症状的完整序：前一轮 bash → 最后一轮 say（首行含 markdown）+
/// task 派发。`ToolStart(task)` 不触发 Say finalize，`SubagentStart`
/// 才 finalize——头部必须是渲染后 preview，且正文不复述首行。
#[test]
fn subagent_dispatch_header_renders_markdown_and_body_does_not_repeat() {
    let mut v = ChatView::default();
    v.begin_turn();
    v.apply(&SessionEvent::TextDelta("go".into()));
    v.apply(&SessionEvent::LlmRoundStart { started_at_ms: 1 });
    v.apply(&SessionEvent::ReasoningDelta("t1".into()));
    bash(&mut v, "b1");
    v.apply(&SessionEvent::LlmRoundEnd);

    v.apply(&SessionEvent::LlmRoundStart { started_at_ms: 2 });
    v.apply(&SessionEvent::ReasoningDelta("t2".into()));
    v.apply(&SessionEvent::TextDelta(
        "**派个 subagent** 去查\n- item2".into(),
    ));
    task_start(&mut v, "t1");
    subagent_start(&mut v);

    let flat = lines(&v);
    // 头部：渲染后的首行文本（无 `**`），正文从渲染口径跳过首行，
    // 只渲染 markdown 自带的分隔空行 + 其余行。
    assert_eq!(
        flat,
        vec![
            "\u{276f} Say:",
            "    go",
            "\u{25b8} Say(2 steps): 派个 subagent 去查",
            "",
            "    ",
            "    \u{2022} item2",
            "\u{2937} subagent [explore] p \u{280b} running, 0 Steps 0s [\u{2192} view]",
        ],
        "dispatch pair: {flat:?}"
    );
}

/// 流式窗口（`ToolStart(task)` 已到、`SubagentStart` 未到）：正文按
/// raw 行流式渲染，首行与 raw preview 相等被跳过——同样没有复述。
#[test]
fn subagent_dispatch_live_window_streams_raw_without_duplication() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think".into()));
    v.apply(&SessionEvent::TextDelta("**去查**\n- item2".into()));
    task_start(&mut v, "t1");

    let flat = lines(&v);
    assert_eq!(
        flat,
        vec![
            "\u{25b8} Say(1 step): **去查**  \u{280b} running ",
            "",
            "    - item2",
        ],
        "live window keeps raw streaming rows, first line skipped: {flat:?}"
    );
}

/// 单行 markdown Say：整块隐藏（头部即全部），不给头部下方留任何
/// 形态的复述或空行串。
#[test]
fn subagent_dispatch_single_line_markdown_say_hides_body() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think".into()));
    v.apply(&SessionEvent::TextDelta("**派个 subagent**".into()));
    task_start(&mut v, "t1");
    subagent_start(&mut v);

    let flat = lines(&v);
    assert_eq!(
        flat,
        vec![
            "\u{25b8} Say(1 step): 派个 subagent",
            "",
            "\u{2937} subagent [explore] p \u{280b} running, 0 Steps 0s [\u{2192} view]",
        ],
        "single-line markdown say: header only, no echo below: {flat:?}"
    );
}
