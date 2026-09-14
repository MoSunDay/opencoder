//! Repro hunt: Say block stays `done:false` (raw, un-markdown-rendered) after
//! the turn's terminal events.
use super::*;

fn assert_all_says_done(v: &ChatView, ctx: &str) {
    for (i, b) in v.blocks.iter().enumerate() {
        if let ChatBlock::Assistant { raw, done, .. } = b {
            assert!(
                done,
                "{ctx}: block {i} Assistant still OPEN raw={raw:?}\nblocks={:#?}",
                v.blocks
            );
        }
    }
}

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

fn round(v: &mut ChatView, rng: &mut Rng, r: usize, last: bool) {
    v.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 1000,
    });
    if rng.below(2) == 0 {
        v.apply(&SessionEvent::ReasoningDelta(format!("think r{r} ")));
    }
    let mut streamed = String::new();
    let chunks = 1 + rng.below(4) as usize;
    for c in 0..chunks {
        let t = format!("r{r}c{c} **bold** `code`\n");
        streamed.push_str(&t);
        if rng.below(4) == 0 {
            continue;
        } // shed delta
        v.apply(&SessionEvent::TextDelta(t));
    }
    let tools = if last { 0 } else { 1 + rng.below(2) as usize };
    for t in 0..tools {
        v.apply(&SessionEvent::ToolStart {
            id: format!("t{r}-{t}"),
            name: "read".into(),
            input: serde_json::json!({}),
        });
        v.apply(&SessionEvent::ToolEnd {
            id: format!("t{r}-{t}"),
            name: "read".into(),
            output: "ok".into(),
            is_error: false,
            images: Vec::new(),
        });
    }
    v.apply(&SessionEvent::LlmRoundEnd);
    if last {
        v.reconcile_completed_assistant(&format!("r{r} FINAL **bold** `code`\nsecond line\n"));
        v.apply(&SessionEvent::Done);
    }
}

#[test]
fn fuzz_say_finalized_after_turn_end() {
    for seed in 1..=500u64 {
        let mut rng = Rng(seed | 1);
        let mut v = ChatView::default();
        let turns = 1 + rng.below(2) as usize;
        for t in 0..turns {
            submit(&mut v, &format!("p{t}"));
            let rounds = 1 + rng.below(3) as usize;
            for r in 0..rounds {
                round(&mut v, &mut rng, r, r == rounds - 1);
            }
            v.finalize_assistant(); // TurnDone safety net
            assert_all_says_done(&v, &format!("seed {seed} turn {t}"));
        }
    }
}
