//! Fuzz of the app-loop display-cache gating (app.rs loop-top semantics):
//! after a turn's terminal events quiesce, the last RENDERED frame must
//! reflect the final chat state — no frozen raw streaming view.
thread_local! { static CURRENT_VERSION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }

/// Faithful reduction of app.rs's loop-top cache/render gating.
struct Gate {
    running: bool,
    dirty: bool,
    render_pending: bool,
    body_pending: bool,
    skip: bool,
    cache: Option<u64>,
    last_rendered: Option<u64>,
}

impl Gate {
    fn new() -> Self {
        Self {
            running: true,
            dirty: true,
            render_pending: true,
            body_pending: true,
            skip: false,
            cache: None,
            last_rendered: None,
        }
    }
    fn iter_top(&mut self) {
        if self.dirty && (self.body_pending || self.cache.is_none()) {
            self.cache = Some(CURRENT_VERSION.with(|c| c.get()));
            self.body_pending = false;
        }
        if self.dirty && self.render_pending {
            if !self.skip {
                self.last_rendered = self.cache;
            }
            self.dirty = false;
        }
        self.render_pending = false;
        self.skip = false;
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

#[derive(Clone, Copy, PartialEq)]
enum Step {
    Evt,
    Frame,
    Body,
    Anim,
}

/// Terminal event stream of a turn: visible Say-finalizing events; `running`
/// flips off at Done (fold's Done arm); AssistantFinal and TurnDone follow.
const EVENTS: &[(bool, bool)] = &[
    (true, false), // LlmRoundEnd (say done+rendered in state)
    (true, false), // LlmUsage
    (true, true),  // Done (running -> false)
    (true, false), // AssistantFinal
    (true, false), // TurnDone
];

fn run_seed(seed: u64, log: &mut Vec<String>) -> (Option<u64>, Option<u64>) {
    CURRENT_VERSION.with(|c| c.set(0));
    let mut rng = Rng(seed | 1);
    let mut g = Gate::new();
    let mut ev_i = 0usize;
    let mut frame_due = 0u64;
    let mut body_due = 0u64;
    let mut anim_due = 0u64;
    let mut t = 0u64;
    while ev_i < EVENTS.len() || t < 40 {
        let mut ready: Vec<Step> = Vec::new();
        if ev_i < EVENTS.len() {
            ready.push(Step::Evt);
        }
        if t >= frame_due {
            ready.push(Step::Frame);
        }
        if t >= body_due {
            ready.push(Step::Body);
        }
        if g.running && t >= anim_due {
            ready.push(Step::Anim);
        }
        if ready.is_empty() {
            t += 1;
            continue;
        }
        let step = ready[rng.below(ready.len() as u64) as usize];
        let before = format!(
            "t={t} v={} run={} d={} rp={} bp={} c={:?} lr={:?}",
            CURRENT_VERSION.with(|c| c.get()),
            g.running,
            g.dirty,
            g.render_pending,
            g.body_pending,
            g.cache,
            g.last_rendered
        );
        g.iter_top();
        match step {
            Step::Evt => {
                let (visible, flips) = EVENTS[ev_i];
                CURRENT_VERSION.with(|c| c.set(c.get() + 1));
                ev_i += 1;
                if flips {
                    g.running = false;
                }
                if !g.running {
                    g.body_pending = true;
                }
                g.dirty = true;
                g.skip = !visible;
                log.push(format!(
                    "EVT {before} -> d=1 bp={} skip={}",
                    g.body_pending, g.skip
                ));
            }
            Step::Frame => {
                g.render_pending = true;
                frame_due = t + 1;
                log.push(format!("FRM {before}"));
            }
            Step::Body => {
                g.body_pending = true;
                body_due = t + 3;
                log.push(format!("BDY {before}"));
            }
            Step::Anim => {
                g.dirty = true;
                anim_due = t + 2;
                log.push(format!("ANM {before}"));
            }
        }
        t += 1;
    }
    for _ in 0..10 {
        g.iter_top();
        g.render_pending = true;
    }
    (g.last_rendered, g.cache)
}

/// Pure gate lock (no fuzz): with a populated cache, a dirty+render-pending
/// turn whose body flag is NOT set renders the STALE cached frame — this is
/// exactly the frozen-first-frame the clear-confirm/reset paths used to hit
/// (echo/marker pushed but only `dirty` set). Setting `body_pending` on the
/// same loop turn refreshes the cache BEFORE the render, so the first frame
/// already shows the new body. Locks the gate against regression.
#[test]
fn body_pending_flag_refreshes_cache_before_the_same_turn_render() {
    CURRENT_VERSION.with(|c| c.set(1));
    let mut g = Gate::new();
    g.running = false;
    g.body_pending = true;
    g.iter_top(); // primes the cache at version 1
    assert_eq!(g.last_rendered, Some(1));

    // Body changed (version 2) with only dirty+render_pending set: the gate
    // renders the STALE cache — the regression shape.
    CURRENT_VERSION.with(|c| c.set(2));
    g.dirty = true;
    g.render_pending = true;
    g.iter_top();
    assert_eq!(
        g.last_rendered,
        Some(1),
        "without body_pending the stale cached frame renders"
    );

    // Same situation, but the change ALSO set body_pending (the fix): the
    // cache refreshes before the render — the new frame lands immediately.
    CURRENT_VERSION.with(|c| c.set(3));
    g.dirty = true;
    g.render_pending = true;
    g.body_pending = true;
    g.iter_top();
    assert_eq!(g.cache, Some(3), "body_pending refreshes the cache");
    assert_eq!(
        g.last_rendered,
        Some(3),
        "and the same-turn render shows the refreshed body"
    );
}

#[test]
fn fuzz_last_frame_matches_final_state_after_turn_end() {
    for seed in 1..=4000u64 {
        let mut log = Vec::new();
        let (last, cache) = run_seed(seed, &mut log);
        assert_eq!(
            last,
            Some(EVENTS.len() as u64),
            "seed {seed}: last={last:?} cache={cache:?}\n{}",
            log.iter()
                .rev()
                .take(30)
                .rev()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
