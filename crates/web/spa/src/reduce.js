// reduce.js — pure transcript state reducers. No React, no fetch, no DOM:
// vitest exercises them directly. Two entry points:
//   * turnsFromMessages(messages) — flatten a GET /api/sessions/:id snapshot
//     (Message { role, blocks: ContentBlock[] }) into render turns;
//   * reduceFrame(state, frame, nowMs) — fold one SSE frame into the live
//     stream state. Both return NEW state objects (no mutation).

/// Payload of text/reasoning deltas: the server (runner/event.rs) uses `text`;
/// node worker uploads may carry `delta` (node_protocol.rs) — accept both.
export function deltaTextOf(data) {
  const d = data || {};
  const v = d.text !== undefined ? d.text : d.delta;
  return typeof v === 'string' ? v : '';
}

/// A `subagent_child` frame's `event` field is the nested SessionEvent in
/// serde's externally tagged form (`{"TextDelta": "..."}`), NOT the SSE wire
/// form (`{event: 'text_delta', data: {...}}`). Convert: PascalCase variant →
/// snake_case event name; newtype string payloads (TextDelta / Status /
/// Error / CompactionDelta / ReasoningDelta) wrap into the `{text}` /
/// `{error}` object shape the SSE payloads use, so reduceFrame can fold them
/// unchanged. Returns null for anything unrecognizable.
export function nestedEventOf(raw) {
  if (!raw || typeof raw !== 'object') {
    return null;
  }
  const keys = Object.keys(raw);
  if (keys.length !== 1) {
    return null;
  }
  const event = keys[0].replace(/([a-z0-9])([A-Z])/g, '$1_$2').toLowerCase();
  // Newtype string variants wrap into the field name their SSE payload uses.
  const stringField = {
    text_delta: 'text', reasoning_delta: 'text', compaction_delta: 'text',
    status: 'status', error: 'error',
  }[event];
  const payload = stringField && typeof raw[keys[0]] === 'string'
    ? { [stringField]: raw[keys[0]] }
    : raw[keys[0]];
  return { event, data: payload || {} };
}

/// Index of the open subagent turn for tool-call `id` (last match wins — ids
/// are unique per run, but a resumed transcript replays them in order).
function subagentIndex(turns, id) {
  if (!id) {
    return -1;
  }
  for (let i = turns.length - 1; i >= 0; i -= 1) {
    if (turns[i].kind === 'subagent' && turns[i].id === id) {
      return i;
    }
  }
  return -1;
}

export function emptyStream() {
  return { turns: [], usage: null, status: 'idle', error: null };
}

function blockToTurns(role, b) {
  if (!b || typeof b !== 'object') {
    return [];
  }
  // Wire blocks are serde-tagged `kind` (core Message ContentBlock); older
  // fixtures used `type`. Accept both — mismatch silently produced EMPTY
  // transcripts from every store snapshot (real-browser acceptance).
  const kind = b.kind || b.type;
  if (kind === 'text') {
    return [{ kind: 'text', role, text: b.text || '' }];
  }
  if (kind === 'reasoning') {
    return [{ kind: 'think', role, text: b.text || '' }];
  }
  if (kind === 'tool_use') {
    return [{ kind: 'tool', role, id: b.id || b.tool_use_id || null, name: b.name || 'tool', input: fmtValue(b.input), output: null, isError: false, durationMs: null }];
  }
  if (kind === 'tool_result') {
    const out = b.output !== undefined && b.output !== null ? b.output : (b.content || []);
    return [{ kind: 'tool', role, id: b.tool_use_id || b.id || null, name: 'result', input: null, output: fmtValue(out), isError: !!b.is_error, durationMs: null }];
  }
  if (kind === 'image' || kind === 'image_url') {
    return [{ kind: 'text', role, text: '[image]' }];
  }
  return [];
}

/// Snapshot messages → turns. A tool_result block attaches to the open
/// tool_use turn before it when one exists (same visual row, like the TUI).
export function turnsFromMessages(messages) {
  const turns = [];
  const list = Array.isArray(messages) ? messages : [];
  for (const m of list) {
    const role = (m && m.role) || 'assistant';
    // Echo contract (mirrors the TUI replay): synthetic user messages are
    // internal and skipped, UNLESS they carry a verbatim `display` text —
    // skill triggers record the raw `$name` input there. Real user turns
    // prefer `display` over the recorded blocks so the transcript shows the
    // user's input verbatim (`$skill` tokens included), never the resolved
    // clean text the LLM consumes.
    if (role === 'user' && m && m.synthetic && !m.display) {
      continue;
    }
    const displayText = role === 'user' && m && typeof m.display === 'string' && m.display !== ''
      ? m.display
      : null;
    for (const b of (m && m.blocks) || []) {
      const bkind = (b && (b.kind || b.type)) || '';
      if (bkind === 'tool_result') {
        const open = findOpenTool(turns, b.tool_use_id || b.id);
        if (open) {
          turns[open] = { ...turns[open], output: fmtValue(b.output !== undefined && b.output !== null ? b.output : b.content), isError: !!b.is_error };
          continue;
        }
      }
      if (displayText && bkind === 'text') {
        turns.push({ kind: 'text', role, text: displayText });
        continue;
      }
      turns.push(...blockToTurns(role, b));
    }
  }
  return turns;
}

function findOpenTool(turns, id) {
  for (let i = turns.length - 1; i >= 0; i -= 1) {
    const t = turns[i];
    if (t.kind !== 'tool') {
      continue;
    }
    if (t.output === null && (!id || !t.id || t.id === id)) {
      return i;
    }
  }
  return -1;
}

function fmtValue(v) {
  if (v === undefined || v === null) {
    return '';
  }
  if (typeof v === 'string') {
    return v;
  }
  try {
    return JSON.stringify(v, null, 2);
  } catch {
    return String(v);
  }
}

function num(v) {
  return typeof v === 'number' && Number.isFinite(v) ? v : null;
}

function withTurns(state, turns) {
  return { ...state, turns };
}

function appendDelta(state, kind, text) {
  if (!text) {
    return state;
  }
  const turns = state.turns.slice();
  const last = turns[turns.length - 1];
  if (last && last.kind === kind && last.role === 'assistant' && (kind === 'think' || last.open)) {
    turns[turns.length - 1] = { ...last, text: (last.text || '') + text };
  } else {
    turns.push(kind === 'think'
      ? { kind: 'think', role: 'assistant', text }
      : { kind: 'text', role: 'assistant', text, open: true });
  }
  return withTurns(state, turns);
}

function closeOpenText(turns) {
  const last = turns[turns.length - 1];
  if (last && last.kind === 'text' && last.open) {
    const copy = turns.slice();
    copy[copy.length - 1] = { ...last, open: false };
    return copy;
  }
  return turns;
}

/// Snapshot messages → aggregated usage for the footer. Store rows carry
/// per-message usage; a reloaded console has no llm_usage frame to remember,
/// so sum the rows (all-zero/absent → null, no empty footer).
export function usageFromMessages(messages) {
  const list = Array.isArray(messages) ? messages : [];
  let input = 0;
  let output = 0;
  let seen = false;
  for (const m of list) {
    const u = (m && m.usage) || {};
    const i = num(u.input_tokens) || 0;
    const o = num(u.output_tokens) || 0;
    if (i > 0 || o > 0) {
      seen = true;
    }
    input += i;
    output += o;
  }
  if (!seen) {
    return null;
  }
  return { input, output, total: input + output, contextWindow: null };
}

function usageOf(data) {
  const d = data || {};
  const input = num(d.input_tokens) || 0;
  const output = num(d.output_tokens) || 0;
  const total = num(d.total_tokens) || input + output;
  const window = num(d.context_window) || num(d.context_window_tokens) || num(d.max_context_tokens);
  return { input, output, total, contextWindow: window };
}

/// Fold one SSE frame ({event, data}) into the stream state. `nowMs` is
/// injected (purity): tool duration falls back to arrival-time delta when the
/// frames carry no duration field (none do today — verified in runner/event.rs).
export function reduceFrame(state, frame, nowMs) {
  const event = frame && frame.event;
  const data = (frame && frame.data) || {};
  switch (event) {
    case 'text_delta':
      return appendDelta(state, 'text', deltaTextOf(data));
    case 'reasoning_delta':
      return appendDelta(state, 'think', deltaTextOf(data));
    case 'tool_start':
    case 'tool_update': {
      const turns = closeOpenText(state.turns.slice());
      turns.push({
        kind: 'tool',
        role: 'assistant',
        id: data.id || data.tool_use_id || null,
        name: data.name || 'tool',
        input: fmtValue(data.input),
        output: null,
        isError: false,
        durationMs: num(data.duration_ms) || num(data.duration),
        startedAt: nowMs,
      });
      return withTurns(state, turns);
    }
    case 'tool_end': {
      const turns = state.turns.slice();
      const idx = findOpenTool(turns, data.id || data.tool_use_id);
      const out = fmtValue(data.output !== undefined ? data.output : data.content);
      const isErr = !!data.is_error;
      const dur = num(data.duration_ms) || num(data.duration);
      if (idx >= 0) {
        const t = turns[idx];
        turns[idx] = {
          ...t,
          name: t.name === 'result' ? (data.name || t.name) : t.name,
          output: out,
          isError: isErr || t.isError,
          durationMs: dur || (t.startedAt ? Math.max(0, nowMs - t.startedAt) : null),
        };
      } else {
        turns.push({ kind: 'tool', role: 'assistant', id: data.id || null, name: data.name || 'result', input: null, output: out, isError: isErr, durationMs: dur });
      }
      return withTurns(state, turns);
    }
    case 'llm_usage':
      return { ...state, usage: usageOf(data) };
    case 'queue_consumed':
    case 'steer_consumed': {
      const text = typeof data.text === 'string' ? data.text : '';
      if (!text) {
        return state;
      }
      const turns = closeOpenText(state.turns.slice());
      turns.push({ kind: 'text', role: 'user', text });
      return withTurns(state, turns);
    }
    case 'status': {
      const text = typeof data.status === 'string' ? data.status : '';
      return text ? withTurns(state, state.turns.concat([{ kind: 'sys', text }])) : state;
    }
    case 'compaction': {
      const summary = typeof data.summary === 'string' ? data.summary : 'compacted';
      return withTurns(state, closeOpenText(state.turns.slice()).concat([{ kind: 'sys', text: '🗜 ' + summary }]));
    }
    case 'agent_switched':
      return withTurns(state, state.turns.concat([{ kind: 'sys', text: 'agent → ' + String(data.agent || '') }]));
    case 'model_switched':
      return withTurns(state, state.turns.concat([{ kind: 'sys', text: 'model → ' + String(data.model || '') }]));
    case 'subagent_start': {
      // Foldable block per subagent (TUI chat.rs parity), keyed by the
      // tool-call id; `subagent_child` frames fold into it via reduceFrame.
      const turn = {
        kind: 'subagent',
        id: data.id || null,
        name: data.kind || 'subagent',
        prompt: typeof data.prompt === 'string' ? data.prompt : '',
        childSessionId: data.child_session_id || null,
        status: 'running',
        ok: null,
        summary: null,
        events: [],
        usage: null,
        startedAt: nowMs,
      };
      return withTurns(state, closeOpenText(state.turns.slice()).concat([turn]));
    }
    case 'subagent_child': {
      const idx = subagentIndex(state.turns, data.id);
      if (idx < 0) {
        return state;
      }
      const nested = nestedEventOf(data.event);
      if (!nested) {
        return state;
      }
      const turns = state.turns.slice();
      const t = turns[idx];
      const child = reduceFrame(
        { turns: t.events, usage: t.usage, status: t.status, error: null },
        nested,
        nowMs,
      );
      turns[idx] = {
        ...t,
        events: child.turns,
        usage: child.usage || t.usage,
        status: child.status === 'done' || child.status === 'error' ? child.status : t.status,
      };
      return withTurns(state, turns);
    }
    case 'subagent_end': {
      const idx = subagentIndex(state.turns, data.id);
      if (idx < 0) {
        return state;
      }
      const turns = state.turns.slice();
      turns[idx] = {
        ...turns[idx],
        status: data.cancelled ? 'cancelled' : (data.ok ? 'done' : 'error'),
        ok: !!data.ok && !data.cancelled,
        summary: typeof data.summary === 'string' ? data.summary : null,
      };
      return withTurns(state, turns);
    }
    case 'compaction_delta': {
      const text = typeof data.text === 'string' ? data.text : '';
      return text ? withTurns(state, state.turns.concat([{ kind: 'sys', text: '🗜 ' + text }])) : state;
    }
    case 'autopilot': {
      const phase = String(data.phase || '').toLowerCase();
      const it = Number.isFinite(data.iteration) ? data.iteration : null;
      return withTurns(state, state.turns.concat([{ kind: 'sys', text: '🛸 autopilot ' + phase + (it !== null ? ' #' + it : '') }]));
    }
    case 'interrupted':
      return withTurns(state, state.turns.concat([{ kind: 'sys', text: '⏹ interrupted' }]));
    case 'transcript_reset':
      // Wire payload is `{}` (runner/event.rs) — the collapsed transcript
      // cannot be rebuilt from the frame. chat.jsx reacts to this event by
      // re-fetching the store snapshot (same path as `done`); here we only
      // close the open text turn so a lost reload still renders sanely.
      return { ...state, turns: closeOpenText(state.turns.slice()) };
    case 'done':
      return { ...state, status: 'done', turns: closeOpenText(state.turns.slice()) };
    case 'error': {
      // A lag-marked error (api.rs map_broadcast_result) is a consumer
      // re-sync signal, not a run failure: sse.js restarts from the persisted
      // head while the run keeps producing. Folding it terminal would latch
      // status 'error' mid-run — and, since chat.jsx releases busy on terminal
      // errors, would free the composer for a still-running turn.
      if (data && Number.isFinite(data.lag)) {
        return state;
      }
      return { ...state, status: 'error', error: String(data.error || data.message || 'error'), turns: closeOpenText(state.turns.slice()) };
    }
    default:
      return state;
  }
}

// Control command heads (crates/session/src/control_cmd.rs::split_control_prefix).
const CONTROL_HEADS = new Set(['/act', '/plan', '/act_clear_context', '/clear_context']);

/// Mirror of crates/session/src/control_cmd.rs::consumed_echo_text — the one
/// echo contract: a compound control submission echoes only its tail, a bare
/// control command echoes nothing, plain text echoes verbatim.
export function consumedEchoText(prompt) {
  const text = typeof prompt === 'string' ? prompt : '';
  const trimmed = text.trim();
  const head = trimmed.split(/\s+/)[0] || '';
  if (!CONTROL_HEADS.has(head)) {
    return text;
  }
  return trimmed.slice(head.length).trim();
}

/// User prompt echo for flows where the server emits no echo frame (remote
/// dispatch has no queue_consumed — the task session is synthetic). Feed it
/// through consumedEchoText so the optimistic bubble matches what the server
/// would have recorded — never the command token itself.
export function withUserTurn(state, text) {
  if (!text) {
    return state;
  }
  return withTurns(state, closeOpenText(state.turns.slice()).concat([{ kind: 'text', role: 'user', text }]));
}
