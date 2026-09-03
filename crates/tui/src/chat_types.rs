use ratatui::text::{Line, Span};

/// Braille spinner frames shown next to a running subagent header. Matches the
/// status-bar spinner in `render.rs` so the UI has one consistent motion.
pub(super) const SPINNER: [&str; 10] = [
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}", "\u{2827}",
    "\u{2807}", "\u{280f}",
];

/// Maximum number of lines captured from a single tool-result event. Even
/// when the user expands a tool-output block, memory and per-refresh
/// `flatten_with` cost are bounded. Collapsed mode still renders only the
/// header.
pub(crate) const TOOL_OUTPUT_LINES: usize = 200;

/// A single visual block in the transcript. Replaces the flat `Vec<Line>`
/// model so we can have collapsible thinking blocks, streaming-vs-rendered
/// assistant text, and tool blocks with structured output.
/// UI transcript model held in `Vec<ChatBlock>` (heap-allocated). The
/// `Subagent` variant deliberately nests a `ChatView` (which itself holds
/// `Vec<ChatBlock>`) to fold subagent transcripts; this recursion makes one
/// variant large by design.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum ChatBlock {
    /// User prompt, queued/steer marker, system notice — plain styled lines.
    Marker(Vec<Line<'static>>),
    /// User-submitted message. Non-streaming: the markdown body is
    /// pre-rendered at submit time and held in `rendered`. Mirrors the
    /// `Assistant` block's structure so both share the `❯ label:` +
    /// 4-space-indented body layout.
    User { rendered: Vec<Line<'static>> },
    /// Assistant text output. While streaming (`done == false`) the raw text is
    /// shown as plain lines for low latency. On turn completion the text is
    /// rendered as markdown exactly once, then `done` flips to `true`.
    Assistant {
        raw: String,
        rendered: Vec<Line<'static>>,
        done: bool,
    },
    /// Collapsible reasoning/thinking block (plain text, click-to-expand).
    Thinking {
        text: String,
        collapsed: bool,
        sealed: bool,
    },
    /// Collapsible compaction-summary block (plain text, click-to-expand).
    /// Mirrors Thinking: click-to-expand. While `streaming` is true the block
    /// is expanded and its `text` grows with each `CompactionDelta` so the user
    /// sees the summary as it is generated. The final `Compaction(summary)`
    /// event finalizes it (full text + collapsed).
    Compaction {
        text: String,
        collapsed: bool,
        streaming: bool,
    },
    /// One assistant turn's tool activity, folded into a three-level
    /// drill-down ladder: turn → step → function call.
    /// Every user input (submit, consumed steer, consumed queued prompt)
    /// owns one group anchored below its echo; intervening Say,
    /// image, marker, or subagent blocks do not split it. A Step is Thinking
    /// plus every function call until the next Thinking run. The group renders ONE
    /// clickable `▸ N Steps` row at col 0 (collapsed by default); opening it
    /// lists the step rows, opening a step reveals its thinking plus a
    /// `N Function calls` aggregation row, and opening that row reveals the
    /// individual calls. Opening a call reveals only that call's result.
    /// `progress_active` drives the group-row animation until Say begins and
    /// cannot be reactivated after non-empty Say output exists.
    /// Ctrl+L resets every disclosure level.
    StepGroup {
        steps: Vec<Step>,
        open: bool,
        progress_active: bool,
    },
    /// Inline image attachment rendered as half-block ASCII art.
    /// `filename` is the display name; `rendered` is the pre-computed
    /// half-block `Line` set (empty when rendering failed → placeholder).
    Image {
        filename: String,
        rendered: Vec<Line<'static>>,
    },
    /// Foldable subagent block. Clicking the header enters the subagent's
    /// perspective (ctx-switch) showing its child `view` as the full body plus
    /// its own context stats. The header always renders as a single clickable
    /// line with running/done/failed status — no inline expansion.
    Subagent {
        id: String,
        child_session_id: String,
        kind: String,
        prompt: String,
        view: ChatView,
        done: bool,
        ok: bool,
        cancelled: bool,
        summary: String,
        started_at_ms: i64,
        elapsed_ms: Option<u64>,
    },
    /// Read-only context card. Rendered on replay from the persisted
    /// `handoff_plan` meta (plan text, or a clear-context seed's preserved
    /// reply). `raw` holds the original markdown source so the plan editor
    /// can edit it; `rendered` is the pre-computed markdown rendering for
    /// display. Purely informational — never an input block.
    Plan {
        rendered: Vec<Line<'static>>,
        raw: String,
    },
}

/// The `/sidecar` Q/A panel state, held OUTSIDE `blocks` (field
/// `ChatView::sidecar`). It is ephemeral display state, not a transcript
/// block: it must never perturb the streaming invariants on `blocks`
/// (reasoning/text delta tail-merging, tool-group tail merge, finalize).
/// Display-only — sidecar frames are never persisted; the conversation's
/// cost reaches the durable layer via bare `LlmUsage`. The panel renders
/// ZERO lines in the flat transcript (the focused body is swapped in by
/// `compute_display`; `chat::sidecar::purge` clears the field on exit).
#[derive(Default, Clone, Debug, PartialEq)]
pub struct SidecarPanel {
    pub id: String,
    pub question: String,
    /// The panel's own nested transcript. `Box` breaks the type recursion
    /// (the panel is held by value in `ChatView::sidecar`).
    pub view: Box<ChatView>,
    pub done: bool,
    pub ok: bool,
    pub answer: Option<String>,
    pub total_tokens: u64,
    pub rounds: u32,
    pub started_at_ms: i64,
    pub elapsed_ms: u64,
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct ChatView {
    pub blocks: Vec<ChatBlock>,
    pub agent: String,
    pub status: String,
    /// Start of the currently-running provider/model round. Display-only:
    /// this value never becomes a persisted model message or prompt content.
    /// While `Some`, `[call cost]` counts up live. Cleared on `LlmRoundEnd`
    /// (transferring the final value to `frozen_round_ms`), `Done`, `Error`.
    pub llm_round_started_at_ms: Option<i64>,
    /// Frozen final elapsed of the most recent completed LLM round. Set by
    /// `LlmRoundEnd`, cleared by the next `LlmRoundStart` (reset), `Done`,
    /// `Error`. While `llm_round_started_at_ms` is `None` and this is `Some`,
    /// `[call cost]` holds the frozen value so the timer stays visible during
    /// inter-round tool execution instead of disappearing.
    pub frozen_round_ms: Option<u64>,
    /// Session-lifetime real token consumption, accumulated from
    /// `LlmUsage` events (provider-reported `total_tokens`, one per
    /// assistant message that carried usage) plus replayed message usage.
    /// INCLUDES subagent consumption: live rounds arrive wrapped in
    /// `SubagentChild` and are added here as well as into the child view;
    /// replay adds each reconstructed child view's total. Focused child
    /// views still show only their own spend. Display-only: drives the
    /// bottom-left `[tok cost]` corner label.
    pub tokens_total: u64,
    /// Provider-truth context size of the most recent completed LLM round
    /// (`total_tokens` from the latest `LlmUsage`, verbatim). When `Some`,
    /// the status bar's `ctx (used/limit)` shows this — there is no
    /// local-estimate fallback, so `None` (no usage-carrying round yet)
    /// renders `—` and 0%. Kept stale across `ModelSwitch` / `Compaction`
    /// / `TranscriptReset` until the next round reports fresh usage; resume
    /// rebuilds it from persisted message usage. Never set from subagent
    /// rounds: a child's context is not part of this view's window.
    pub real_context_tokens: Option<u64>,
    /// First block belonging to the currently admitted top-level turn. A
    /// reliable completed-text event uses this floor to repair any parent
    /// `TextDelta` chunks dropped by the bounded worker channel without ever
    /// overwriting an Assistant block from an earlier turn.
    pub turn_block_start: usize,
    /// Verbatim echo of the currently admitted turn's user input (submit,
    /// steer consumption, or queue consumption). `TranscriptReset` rebuilds
    /// the whole view from the folded transcript, which lands BEFORE the
    /// in-flight prompt is recorded — without this memory the user boundary
    /// of the running turn would be wiped and its ladder would render
    /// orphaned (the "steps of the next turn glued onto the previous one"
    /// regression). `rebuild_after_reset` re-pushes it below the rebuilt
    /// blocks; cleared on `Done`/`Error`.
    pub pending_turn_echo: Option<String>,
    /// Whether the user has submitted at least one prompt since session start
    /// (or last TranscriptReset). Gates the in-body tutorial: the welcome text
    /// hides once the user has interacted, even if the first submission was a
    /// bare control command that does not add a transcript block.
    pub submitted: bool,
    /// Pending steer inputs mirrored from the store, owned by this view so the
    /// `SteerConsumed` handler can resolve seq -> prompt text and drop the row
    /// in one place. The single source of truth for steer display state.
    pub steer_items: Vec<(i64, String)>,
    /// Number of subagents currently in flight (SubagentStart seen, no matching
    /// SubagentEnd yet). Surfaced in the status bar as a live "running" badge so
    /// concurrent dispatch is visible.
    pub subagents_running: u32,
    /// Total subagents dispatched this session (running + completed).
    pub subagents_total: u32,
    /// Estimated tokens consumed by this view's own transcript (excludes
    /// child subagent tokens, which live on the child ChatView). Used to
    /// show context stats when viewing a subagent's perspective.
    pub context_used: u64,
    /// Index of the parent's assistant block whose content is withheld while
    /// MULTIPLE subagents are in flight (see issue #5). The block renders zero
    /// lines in `flatten_with` and is excluded from header line-accounting so
    /// hit-rects stay aligned. Cleared once all subagents finish (the content
    /// then appears in one shot).
    pub hidden_assistant_idx: Option<usize>,
    /// Explicitly saved annotation text (from /annotation editor).
    pub annotation_text: Option<String>,
    /// First non-empty, non-slash user prompt — used to prefill the
    /// annotation editor when no explicit annotation has been saved.
    pub first_prompt: Option<String>,
    /// Whether the sidecar interaction box is focused (body swapped for the
    /// sidecar panel's nested view, mode chip reads `sidecar`). Mutually
    /// exclusive with a subagent focus: sidecar Enter routes to the sidecar
    /// actor, never to the parent's steer/queue paths. Display-only state —
    /// never persisted, cleared on `/task` switches.
    pub sidecar_focus: bool,
    /// The `/sidecar` Q/A panel while it is open (never inside `blocks` —
    /// see `SidecarPanel`). Cleared on exit (ESC / Ctrl+L) and on `/task`
    /// switches; `None` plus `sidecar_focus == false` is the closed state.
    pub sidecar: Option<SidecarPanel>,
}

/// Locates a `Thinking` block's header line for mouse hit-testing.
/// `header_line_idx` is the index within `ChatView::flatten()` of the block's
/// header line; `block_idx` is its index in `ChatView::blocks`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThinkingHeader {
    pub block_idx: usize,
    pub header_line_idx: usize,
}

/// Locates a `Subagent` block's header line for mouse hit-testing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SubagentHeader {
    pub block_idx: usize,
    pub header_line_idx: usize,
}

/// One step inside a `ChatBlock::StepGroup`: one Thinking run (as rendered
/// markdown lines) plus every function call before the next Thinking run.
/// The step's row renders only while the enclosing turn is open; its
/// content (thinking + calls aggregation row) is visible only while the
/// step is open; individual call rows additionally require `calls_open`;
/// a single call's result additionally requires `ToolCall::expanded`.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    /// Sanitized source accumulated from live reasoning deltas. Keeping the
    /// source avoids lossy rendered-text round trips (notably trailing
    /// whitespace and Markdown delimiters) before each re-render.
    pub thinking_raw: String,
    /// Rendered markdown of the round's thinking. Empty when the round went
    /// straight to tools. A round with tools NEVER leaves a standalone
    /// `Thinking` block behind — its reasoning is absorbed here (see
    /// `steps::absorb_pending_thinking`).
    pub thinking: Vec<Line<'static>>,
    /// Whether `thinking_raw` has advanced beyond `thinking`. Collapsed
    /// streaming steps only append source; opening the step materializes the
    /// markdown once, avoiding an O(n²) full re-render on every hidden delta.
    pub thinking_dirty: bool,
    pub calls: Vec<ToolCall>,
    /// Whether this step's Thinking content + calls aggregation row render.
    pub open: bool,
    /// Whether this step's individual function-call rows render. Requires
    /// `open`; each call result remains independently collapsible.
    pub calls_open: bool,
    /// Whether this step's streaming thinking has been accounted into
    /// `context_used` already (mirrors the top-level `Thinking::sealed`
    /// flag). Rendered on demand by `steps::render_step_thinking`, sealed once
    /// per step at round finalize.
    pub sealed: bool,
}

/// One tool call inside a step of a `ChatBlock::StepGroup`.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: String,
    /// `▸ name args` header line, shown on the call's row.
    pub header: Line<'static>,
    /// Sanitized output lines, shown while the call is expanded.
    pub output: Vec<Line<'static>>,
    pub started_at_ms: Option<i64>,
    /// `None` while the call is still running — drives call timing/output
    /// state only; call completion is not a Step boundary.
    pub elapsed_ms: Option<u64>,
    /// Show this call's `output` under its header. Requires the enclosing
    /// step to be open. Toggled by clicking the call's header row; reset by
    /// `collapse_all_collapsible`.
    pub expanded: bool,
}

/// Exact text prefixes of a `StepGroup`'s step row (`❯ Step(n)` when the
/// step is open, `▸ Step(n)` when collapsed). Declared here so copy-mode's
/// structured cleaner matches the render shape at compile time.
pub(crate) const STEP_ROW_OPEN_PREFIX: &str = "\u{276f} Step(";
pub(crate) const STEP_ROW_CLOSED_PREFIX: &str = "\u{25b8} Step(";

/// Exact text prefixes shared by the turn row (`❯ N Steps` / `▸ N Steps`)
/// and the per-step calls aggregation row (`❯ N Function calls` /
/// `▸ N Function calls`). The trailing space is part of the prefix.
pub(crate) const GROUP_ROW_OPEN_PREFIX: &str = "\u{276f} ";
pub(crate) const GROUP_ROW_CLOSED_PREFIX: &str = "\u{25b8} ";

/// Locates one clickable row inside a `StepGroup` — the turn row, a step
/// row, a calls aggregation row, or a function-call header row — for mouse
/// hit-testing. `call_idx` is the block's flat *visible-target index*: the
/// row's position among the group's currently rendered rows (see
/// `visible_targets`). Clicking toggles exactly that target (turn fold,
/// step fold, calls list, or single-call result), leaving siblings untouched.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolCallHeader {
    pub block_idx: usize,
    pub call_idx: usize,
    pub header_line_idx: usize,
}

/// Locates a `Compaction` block's header line for mouse hit-testing.
/// Mirrors `ThinkingHeader`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompactionHeader {
    pub block_idx: usize,
    pub header_line_idx: usize,
}

/// Prepend a fixed-width indent to each line's existing spans, producing a
/// new owned `Vec<Line>` suitable for `flatten_with`. Used by the
/// `Assistant`, `User`, and `Image` flatten arms so they share one
/// indented-body implementation.
pub(super) fn indented(rendered: &[Line<'static>], width: usize) -> Vec<Line<'static>> {
    let indent = Span::raw(" ".repeat(width));
    rendered
        .iter()
        .map(|l| {
            let mut spans = vec![indent.clone()];
            spans.extend(l.spans.iter().cloned());
            Line::from(spans)
        })
        .collect()
}
