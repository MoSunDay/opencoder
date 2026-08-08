use ratatui::text::Line;

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
    /// Tool invocation: a header line plus its (full) output lines. `collapsed`
    /// hides the output behind a click-to-expand header, mirroring Thinking.
    Tool {
        id: String,
        header: Line<'static>,
        output: Vec<Line<'static>>,
        collapsed: bool,
        started_at_ms: i64,
        elapsed_ms: Option<u64>,
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
    /// Read-only plan card shown after plan→act handoff. `raw` holds the
    /// original markdown source so it can be edited in plan mode; `rendered`
    /// is the pre-computed markdown rendering for display.
    /// Not interactive post-handoff — purely informational context.
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
    pub llm_round_started_at_ms: Option<i64>,
    /// Whether the user submitted a prompt while in plan mode since the last
    /// plan-mode entry. Reset to `false` on every `AgentSwitch` *to* plan.
    /// Drives the plan→act handoff decision: only hand off when the user
    /// actually interacted with the plan agent, otherwise plain-swap.
    pub plan_submitted: bool,
    /// Deferred arming of `plan_submitted` for a compound `/plan <content>`
    /// submitted while the agent was still `act` (the mode switch lands
    /// asynchronously via `AgentSwitch("plan")`, which otherwise resets the
    /// flag). Set by the submit/steer/queue paths in `app.rs`; consumed by the
    /// `AgentSwitch` handler in `chat.rs` so Shift+Tab after the plan turn
    /// keeps the plan and starts the task.
    pub pending_plan_arm: bool,
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
    /// Explicitly saved requirement text (from /requirement editor).
    pub requirement_text: Option<String>,
    /// First non-empty, non-slash user prompt — used to prefill the
    /// requirement editor when no explicit requirement has been saved.
    pub first_prompt: Option<String>,
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

/// Locates a `Tool` block's header line for mouse hit-testing.
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
