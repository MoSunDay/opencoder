//! 中断→再提交的 Say 多行换行回归：cancel_running_turn 的本地 marker、
//! runner 的 Status("interrupted")/Done、以及下一回合的 push_user/begin_turn
//! 交替出现时，新回合 Say 的多行正文必须逐行渲染（行间分隔不丢）。
use super::*;

fn flat_lines(v: &ChatView) -> Vec<String> {
    v.flatten()
        .into_iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect()
}

fn submit(v: &mut ChatView, text: &str) {
    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render(text),
    });
    v.push_marker(Line::from(""));
    v.begin_turn();
}

/// 复现用户报告：中断后立即再提交，新 Say 的多行正文行贴在一起；
/// 再中断再提交又恢复。驱动与 app 层完全一致的调用序列。
#[test]
fn interrupt_then_resubmit_keeps_multiline_breaks() {
    let mut v = ChatView::default();

    // ── 回合 1：流式中被打断 ──
    submit(&mut v, "do something");
    v.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 1000,
    });
    v.apply(&SessionEvent::TextDelta(
        "first partial line\nsecond partial line".into(),
    ));
    // Ctrl+C：TUI 本地 marker（cancel_running_turn）
    v.push_marker(Line::from("[interrupted] stopping…"));
    // runner 终结事件
    v.apply(&SessionEvent::Status("interrupted".into()));
    v.apply(&SessionEvent::Done);

    // ── 回合 2：立即再提交，多行回答 ──
    submit(&mut v, "retry");
    v.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 2000,
    });
    v.apply(&SessionEvent::TextDelta("alpha\nbeta\ngamma".into()));
    v.apply(&SessionEvent::LlmRoundEnd);
    v.apply(&SessionEvent::Done);

    let ls = flat_lines(&v);
    for (i, l) in ls.iter().enumerate() {
        println!("{:3}|{}", i, l);
    }
    let alpha = ls
        .iter()
        .position(|l| l.contains("alpha"))
        .expect("alpha line");
    assert!(
        ls[alpha + 1].contains("beta"),
        "beta must be its own line, got {ls:?}"
    );
    assert!(
        ls[alpha + 2].contains("gamma"),
        "gamma must be its own line, got {ls:?}"
    );
}

/// 两次中断-再提交的交替稳定性：第 3 回合的换行表现必须与第 2 回合一致。
#[test]
fn repeated_interrupt_resubmit_stays_consistent() {
    let mut v = ChatView::default();
    for round in 0..3 {
        let first = format!("round{} line1", round);
        submit(&mut v, &format!("prompt{}", round));
        v.apply(&SessionEvent::LlmRoundStart {
            started_at_ms: 1000 + round,
        });
        v.apply(&SessionEvent::TextDelta(first));
        v.push_marker(Line::from("[interrupted] stopping…"));
        v.apply(&SessionEvent::Status("interrupted".into()));
        v.apply(&SessionEvent::Done);
    }
    let ls = flat_lines(&v);
    for (i, l) in ls.iter().enumerate() {
        println!("{:3}|{}", i, l);
    }
}

fn call_tool(v: &mut ChatView, id: &str) {
    v.apply(&SessionEvent::ToolStart {
        id: id.into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: id.into(),
        name: "bash".into(),
        output: format!("{id}-out"),
        is_error: false,
        images: Vec::new(),
    });
}

/// 工具回合 + 中断→再提交：合并头部(Say(n steps): 预览)与正文跳行
/// 去重路径下，新回合多行正文必须逐行渲染。
#[test]
fn interrupt_tool_turn_then_resubmit_multiline() {
    let mut v = ChatView::default();

    // ── 回合 1：工具 + 多行 Say，完成后被打断前的完整流 ──
    submit(&mut v, "list files");
    v.begin_turn();
    v.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 1000,
    });
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta(
        "partial tool answer\nline2".into(),
    ));
    // Ctrl+C 中断
    v.push_marker(Line::from("[interrupted] stopping…"));
    v.apply(&SessionEvent::Status("interrupted".into()));
    v.apply(&SessionEvent::Done);

    // ── 回合 2：再提交，工具 + 多行最终回答 ──
    submit(&mut v, "again");
    v.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 2000,
    });
    call_tool(&mut v, "t2");
    v.apply(&SessionEvent::TextDelta(
        "final alpha\nfinal beta\nfinal gamma".into(),
    ));
    v.apply(&SessionEvent::LlmRoundEnd);
    v.reconcile_completed_assistant("final alpha\nfinal beta\nfinal gamma");
    v.apply(&SessionEvent::Done);

    let ls = flat_lines(&v);
    for (i, l) in ls.iter().enumerate() {
        println!("{:3}|{}", i, l);
    }
    // 合并头部行是 `Say(n steps): final alpha` 的预览，正文首行被 Skip
    // 去重；beta/gamma 必须各自成行且不得与 alpha 同行。
    let body_beta = ls
        .iter()
        .position(|l| l.contains("final beta"))
        .expect("beta line");
    let body_gamma = ls
        .iter()
        .position(|l| l.contains("final gamma"))
        .expect("gamma line");
    assert!(body_beta < body_gamma, "beta before gamma, got {ls:?}");
    for l in &ls {
        assert!(
            !(l.contains("final beta") && l.contains("final gamma")),
            "换行丢失(行贴在一起): {l:?}\n全部行: {ls:?}"
        );
    }
}
