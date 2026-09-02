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
    /// A run of consecutive tool calls (one assistant turn's calls between
    /// text/image/marker blocks form one group; any other block splits the
    /// run). Group-level three-state display, cycled by clicking the group
    /// line: Collapsed → List → Results → Collapsed. Ctrl+L resets every
    /// group to Collapsed.
    ToolGroup {
        calls: Vec<ToolCall>,
        state: ToolGroupState,
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
    /// Sidecar Q/A block (`/sidecar <question>`): one block per sidecar
    /// conversation, folding every follow-up turn into the same nested
    /// `view`. Display-only — sidecar frames are never persisted; the
    /// conversation's cost reaches the durable layer via bare `LlmUsage`.
    /// Mirrors `Subagent`'s nested-ChatView shape without the task/registry
    /// semantics: header-only flattening, body visible only while focused
    /// (see `compute_display`'s view swap).
    Sidecar {
        id: String,
        question: String,
        view: ChatView,
        done: bool,
        ok: bool,
        answer: Option<String>,
        total_tokens: u64,
        rounds: u32,
        started_at_ms: i64,
        elapsed_ms: u64,
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

#[derive(Default, Clone, Debug, PartialEq)]
pub struct ChatView {
    pub blocks: Vec<ChatBlock>,
    pub agent: String,
    pub status: String,
    /// Start of the currently-running provider/model round. Display-only:
    /// this value never becomes a persisted model message or prompt content.
    /// While `Some`, `[turn cost]` counts up live. Cleared on `LlmRoundEnd`
    /// (transferring the final value to `frozen_round_ms`), `Done`, `Error`.
    pub llm_round_started_at_ms: Option<i64>,
    /// Frozen final elapsed of the most recent completed LLM round. Set by
    /// `LlmRoundEnd`, cleared by the next `LlmRoundStart` (reset), `Done`,
    /// `Error`. While `llm_round_started_at_ms` is `None` and this is `Some`,
    /// `[turn cost]` holds the frozen value so the timer stays visible during
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
    /// sidecar block's nested view, mode chip reads `sidecar`). Mutually
    /// exclusive with a subagent focus: sidecar Enter routes to the sidecar
    /// actor, never to the parent's steer/queue paths. Display-only state —
    /// never persisted, cleared on `/task` switches.
    pub sidecar_focus: bool,
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

/// Display state of a `ChatBlock::ToolGroup` — the single source of truth for
/// how the group renders (there is no per-call collapse flag to drift).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolGroupState {
    /// One line: `▸ N function calls` (plus a spinner hint while any call is
    /// unfinished). Default for new groups and after Ctrl+L.
    #[default]
    Collapsed,
    /// Group line + each call's header line (tool name + arg summary, no
    /// output).
    List,
    /// List + each call's full output lines.
    Results,
}

/// One tool call inside a `ChatBlock::ToolGroup`.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: String,
    /// `▸ name args` header line, shown in the List/Results states.
    pub header: Line<'static>,
    /// Sanitized output lines, shown in the Results state.
    pub output: Vec<Line<'static>>,
    pub started_at_ms: Option<i64>,
    /// `None` while the call is still running — drives the group line's
    /// running hint.
    pub elapsed_ms: Option<u64>,
}

/// Locates a `ToolGroup` block's group line for mouse hit-testing (clicking
/// cycles the group's display state).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolHeader {
    pub block_idx: usize,
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
