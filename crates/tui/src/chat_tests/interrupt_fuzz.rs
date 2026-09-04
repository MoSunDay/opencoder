//! 模糊式回归：随机交错中断/再提交/多轮 delta/AssistantFinal 的状态序列，
//! 不变量：每个含换行的 Say 正文在展平输出中必须逐行分列（行不粘连）。
use super::*;

fn flat_lines(v: &ChatView) -> Vec<String> {
    v.flatten()
        .into_iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect()
}

/// xorshift64* — 稳定可复现的伪随机序列（不依赖外部 crate）。
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn submit(v: &mut ChatView, text: &str) {
    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render(text),
    });
    v.push_marker(Line::from(""));
    v.begin_turn();
}

/// 单个随机场景：若干「回合」，每回合随机决定：流式块数、是否被打断、
/// 中断后是否还有尾随 delta、是否 reconcile、以及最终多行回答。
fn scenario(seed: u64) {
    let mut rng = Rng(seed | 1);
    let mut v = ChatView::default();
    let rounds = 2 + rng.below(3) as usize; // 2..4
    for r in 0..rounds {
        submit(&mut v, &format!("p{}", r));
        v.apply(&SessionEvent::LlmRoundStart { started_at_ms: 1000 + r as i64 });
        let chunks = 1 + rng.below(3) as usize;
        for c in 0..chunks {
            // 行完整 delta（以 \n 收尾）：块间拼接只会出现在行边界，
            // 这样「单行同时含 l1 与 l2」才唯一对应换行丢失。
            v.apply(&SessionEvent::TextDelta(format!("r{}c{}l1\nr{}c{}l2\n", r, c, r, c)));
        }
        let interrupted = rng.below(2) == 0;
        if interrupted {
            v.push_marker(Line::from("[interrupted] stopping…"));
            // 尾随 delta：取消传播竞态下仍可能到达
            if rng.below(2) == 0 {
                v.apply(&SessionEvent::TextDelta(format!("r{}taill1\nr{}taill2\n", r, r)));
            }
            v.apply(&SessionEvent::Status("interrupted".into()));
            if rng.below(2) == 0 {
                v.reconcile_completed_assistant(&format!("r{}final\nsecond", r));
            }
        } else {
            v.apply(&SessionEvent::LlmRoundEnd);
            v.reconcile_completed_assistant(&format!("r{}final A\nr{}final B\nr{}final C", r, r, r));
        }
        v.apply(&SessionEvent::Done);
    }
    // 不变量：行完整输入下，任何渲染行都不得同时含 l1 与 l2 —— 同时含
    // 即代表两行被拼到了一行（换行丢失）。
    let ls = flat_lines(&v);
    for (i, l) in ls.iter().enumerate() {
        assert!(
            !(l.contains("l1") && l.contains("l2")),
            "seed {seed}: 换行丢失(行贴在一起)于第 {i} 行: {l:?}\n全部行: {ls:?}"
        );
    }
}

#[test]
fn fuzz_interrupt_resubmit_never_glues_lines() {
    for seed in 1..=200u64 {
        scenario(seed);
    }
}
