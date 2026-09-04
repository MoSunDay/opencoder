// reduce.js — pure transcript state reducers. No React, no fetch, no DOM:
// vitest exercises them directly. Two entry points:
//   * turnsFromMessages(messages) — flatten a GET /api/sessions/:id snapshot
//     (Message { role, blocks: ContentBlock[] }) into render turns;
//   * reduceFrame(state, frame, nowMs) — fold one SSE frame into the live
//     stream state. Both return NEW state objects (no mutation).

import {
  absorbSegmentThinking,
  appendStepCall,
  appendStepTurn,
  appendThinkDelta,
  backfillBufferedCall,
  backfillStepsCall,
  clearSayStreaming,
  closeOpenText,
  flushPendingThink,
  markSayStreaming,
  settleTurnProgress,
} from './steps/reducer.js';

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
  // pendingEcho: the in-flight turn's user echo (TUI chat_types.rs
  // pending_turn_echo parity) — remembered from submit/steer/queue echo until
  // done/error, kept across transcript_reset so the store-snapshot rebuild
  // can re-push the user boundary (ensurePendingEcho).
  // applySeq: the persisted-seq watermark — the highest event seq whose
  // effects are already folded into `turns` (null = no seq'd frame folded
  // yet). Resync (sse.js onResync → resyncState) sets it from the /seq head
  // so a replay at/below the watermark can never double-fold; live broadcast
  // frames carry no seq and never move it.
  return { turns: [], usage: null, status: 'idle', error: null, pendingEcho: null, applySeq: null };
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
    // Presentation marker, never a Say: `image:true` keeps it from closing
    // the sub-turn (TUI parity: Image blocks never close a turn).
    return [{ kind: 'text', role, text: '[image]', image: true }];
  }
  return [];
}

/// Snapshot messages → turns. Message-pair semantics now mirror the TUI
/// replay's BLOCK-ORDER walk (replay.rs): one user input owns one or MORE
/// pairs of (n Steps + Say) - a non-empty assistant Text block is a Say that
/// CLOSES the current sub-turn, so its buffered round (and any dangling
/// reasoning) flushes into the ladder ABOVE the Say, and every tool_use
/// after it opens a NEW ladder BELOW it. Empty assistant text blocks vanish
/// like in the replay; image markers render but never close. Reasoning
/// buffers into a pending-think that the next round's first tool_use
/// consumes (cross-message), else folds as a call-less step at the next
/// Say/boundary/walk end. The user/synthetic/display echo contracts are
/// unchanged.
export function turnsFromMessages(messages) {
  const turns = [];
  const list = Array.isArray(messages) ? messages : [];
  let pendingThink = '';
  const blocksOf = (m) => (m && m.blocks) || [];
  for (let mi = 0; mi < list.length; mi += 1) {
    const m = list[mi];
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
    let roundCalls = [];
    let roundThinking = '';
    const blocks = blocksOf(m);
    for (let bi = 0; bi < blocks.length; bi += 1) {
      const b = blocks[bi];
      const bkind = (b && (b.kind || b.type)) || '';
      if (bkind === 'reasoning') {
        pendingThink += (b && b.text) || '';
        continue;
      }
      if (bkind === 'text') {
        const text = typeof (b && b.text) === 'string' ? b.text : '';
        if (role === 'assistant') {
          if (text.trim() === '') {
            // Mirror the TUI replay: empty Say blocks vanish - they neither
            // render nor close the sub-turn.
            continue;
          }
          // A non-empty assistant Text block is a Say: it CLOSES the current
          // sub-turn (TUI replay walks blocks in order - a Text block closes
          // the turn; tool_uses after it belong to the NEXT ladder). Flush
          // the buffered round into the ladder ABOVE the Say first.
          if (roundCalls.length > 0) {
            appendStepTurn(turns, roundThinking || pendingThink, roundCalls);
            roundThinking = '';
            roundCalls = [];
            pendingThink = '';
          } else if (pendingThink) {
            appendStepTurn(turns, pendingThink, []);
            pendingThink = '';
          }
        } else if (pendingThink) {
          // A user text block is a segment boundary: the previous segment's
          // dangling reasoning folds into ITS ladder (floor-aware - below
          // the last Say, above the echo), never into the next segment's
          // round.
          appendStepTurn(turns, pendingThink, []);
          pendingThink = '';
        }
        if (displayText) {
          turns.push({ kind: 'text', role, text: displayText });
          continue;
        }
        turns.push(...blockToTurns(role, b));
        continue;
      }
      if (bkind === 'tool_use') {
        const name = (b && b.name) || 'tool';
        if (name === 'task') {
          // Subagent handle keeps today's flat tool turn.
          turns.push(...blockToTurns(role, b));
          continue;
        }
        // The FIRST tool_use of the message consumes the pending thinking as
        // the step's thinking (coalesce absorbs a trailing thinking run into
        // the next group's first step).
        if (roundCalls.length === 0) {
          roundThinking = pendingThink;
          pendingThink = '';
        }
        roundCalls.push({
          kind: 'tool',
          role,
          id: (b && (b.id || b.tool_use_id)) || null,
          name,
          input: fmtValue(b && b.input),
          output: null,
          isError: false,
          durationMs: null,
          startedAt: null,
        });
        continue;
      }
      if (bkind === 'tool_result') {
        const rid = (b && (b.tool_use_id || b.id)) || null;
        const out = fmtValue(b && b.output !== undefined && b.output !== null ? b.output : b.content);
        const patch = (c) => ({ ...c, output: out, isError: !!(b && b.is_error) });
        // Same-message results land in the round buffer before the step is
        // flushed; older groups backfill by id newest-first; task rows keep
        // the legacy flat path.
        const buffered = backfillBufferedCall(roundCalls, rid, patch);
        if (buffered) {
          roundCalls = buffered;
          continue;
        }
        if (backfillStepsCall(turns, rid, patch)) {
          continue;
        }
        const open = findOpenTool(turns, rid);
        if (open >= 0) {
          turns[open] = { ...turns[open], output: out, isError: !!(b && b.is_error) };
          continue;
        }
      }
      turns.push(...blockToTurns(role, b));
    }
    // End of assistant message: its whole round is ONE step (n calls).
    if (role === 'assistant' && roundCalls.length > 0) {
      appendStepTurn(turns, roundThinking, roundCalls);
    }
  }
  // Trailing reasoning no tool round ever consumes (a pure-text round, or a
  // turn cut off before its Say): a call-less step into the current group,
  // or a NEW group below the last Say via the floor-aware insert.
  if (pendingThink) {
    appendStepTurn(turns, pendingThink, []);
    pendingThink = '';
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

// Terminal/boundary settle: freeze the ladder's progress AND retire every
// Say-row running hint (sayStreaming) so no run outlives the sub-turn —
// done, error, interrupted, transcript_reset, a user echo (queue/steer/
// withUserTurn) all end the currently-streaming Say for good.
function settleTerminal(turns) {
  return clearSayStreaming(settleTurnProgress(turns));
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

// Turn/Step transforms are isolated in `steps/reducer.js` and shared by
// snapshot replay plus live SSE folding.

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
/// Fold one frame into the stream state with the applySeq watermark guard
/// (round-2 #5 resync dedup): a frame that carries a persisted seq at/below
/// `state.applySeq` is already reflected in `turns` — either folded on an
/// earlier connection or covered by a snapshot rebuild — and is dropped
/// verbatim (a replay overlap used to double text, synthesize orphan tool
/// rows and re-push echo turns). Frames without a seq (live broadcasts are
/// emitted before the async event flusher persists them, so they can never
/// carry one) always fold; folding a seq'd frame advances the watermark.
export function reduceFrame(state, frame, nowMs) {
  const seq = frame && Number.isFinite(frame.seq) ? frame.seq : null;
  if (seq !== null && state.applySeq !== null && state.applySeq !== undefined && seq <= state.applySeq) {
    return state;
  }
  const next = foldFrame(state, frame, nowMs);
  return seq === null ? next : { ...next, applySeq: seq };
}

function foldFrame(state, frame, nowMs) {
  const event = frame && frame.event;
  const data = (frame && frame.data) || {};
  switch (event) {
    case 'text_delta': {
      const text = deltaTextOf(data);
      if (!text) {
        return state;
      }
      // Order invariant (reduce.order.test.js): the ladder settles BEFORE
      // the Say lands; markSayStreaming then hands the "running" hint to the
      // Say row (same target turn — it stays "running" until a fresh ladder
      // opens below the Say or a terminal boundary arrives); appendDelta
      // appends/extends the Say LAST.
      return appendDelta(
        withTurns(state, markSayStreaming(settleTurnProgress(state.turns))),
        'text',
        text,
      );
    }
    case 'reasoning_delta': {
      // Streams straight into the steps ladder (think-only step) — a
      // top-level think turn no longer exists on the live path.
      const text = deltaTextOf(data);
      return text ? withTurns(state, appendThinkDelta(state.turns, text)) : state;
    }
    case 'tool_start':
    case 'tool_update': {
      const turns = closeOpenText(state.turns.slice());
      const call = {
        kind: 'tool',
        role: 'assistant',
        id: data.id || data.tool_use_id || null,
        name: data.name || 'tool',
        input: fmtValue(data.input),
        output: null,
        isError: false,
        durationMs: num(data.duration_ms) || num(data.duration),
        startedAt: nowMs,
      };
      if (data.name === 'task') {
        // Subagent handle: keeps TODAY's flat tool turn — the 🤖 subagent
        // block renders the child; task calls never join a step. The round's
        // pending think run folds into the ladder first (no ToolStart can
        // absorb it behind the flat row).
        const flushed = flushPendingThink(turns);
        flushed.push(call);
        return withTurns(state, flushed);
      }
      // Step-ladder fold: any legacy think turn above the last user echo
      // becomes the round's step thinking, then the call joins the ladder
      // the floor-aware helpers locate - the one under the LAST Say when a
      // Say already closed this sub-turn's predecessor (fresh ladder below
      // it), else the turn's own group. Only a later Thinking run opens the
      // next Step.
      const thinking = absorbSegmentThinking(turns);
      appendStepCall(turns, thinking, call, true);
      return withTurns(state, turns);
    }
    case 'tool_end': {
      const out = fmtValue(data.output !== undefined ? data.output : data.content);
      const isErr = !!data.is_error;
      const dur = num(data.duration_ms) || num(data.duration);
      const id = data.id || data.tool_use_id || null;
      const turns = state.turns.slice();
      if (data.name === 'task') {
        // Subagent handle: legacy flat backfill, verbatim.
        const idx = findOpenTool(turns, id);
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
          turns.push({ kind: 'tool', role: 'assistant', id, name: data.name || 'result', input: null, output: out, isError: isErr, durationMs: dur });
        }
        return withTurns(state, turns);
      }
      if (backfillStepsCall(turns, id, (c) => ({
        ...c,
        output: out,
        isError: isErr || c.isError,
        durationMs: dur || (c.startedAt ? Math.max(0, nowMs - c.startedAt) : null),
      }))) {
        return withTurns(state, turns);
      }
      // Orphan end (lost start): synthesize a FINISHED call so the output is
      // kept, folded into the trailing group when one exists — the same fold
      // replay's coalesce applies to adjacent groups.
      appendStepCall(turns, '', {
        kind: 'tool', role: 'assistant', id,
        name: data.name || 'result', input: null,
        output: out, isError: isErr, durationMs: dur ?? 0,
      });
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
      // Hard segment boundary: fold the pre-boundary think run into the
      // ladder before the echo lands (a later tool_start can never cross it).
      const settled = closeOpenText(flushPendingThink(settleTerminal(state.turns)));
      // Optimistic-echo dedup (TUI pending_turn_echo contract): a locally
      // predicted echo (chat.jsx's optimistic flag) and the server's echo
      // frame are the SAME user boundary — when the last turn is that
      // prediction with identical text, fold instead of pushing a second
      // bubble; the flag comes off so the turn is now the authoritative
      // echo. Anything else keeps the plain push below.
      const last = settled.length > 0 ? settled[settled.length - 1] : null;
      if (last && last.kind === 'text' && last.role === 'user'
        && last.text === text && last.optimistic === true) {
        const authoritative = { ...last };
        delete authoritative.optimistic;
        return withTurns({ ...state, pendingEcho: text },
          settled.slice(0, -1).concat([authoritative]));
      }
      const turns = settled.slice();
      turns.push({ kind: 'text', role: 'user', text });
      // Remember the echo (TUI pending_turn_echo): a mid-run transcript_reset
      // rebuilds from a store snapshot that has not recorded it yet.
      return withTurns({ ...state, pendingEcho: text }, turns);
    }
    case 'status': {
      const text = typeof data.status === 'string' ? data.status : '';
      // Status is presentation inside the admitted sub-turn. Fold any legacy
      // top-level think run before placing the marker; the canonical steps
      // item stays discoverable across it until a Say or user echo lands.
      // bubbleItems absorbs the row into the adjacent assistant run so a
      // mid-run badge never splits the [steps … say] bubble.
      return withTurns(state, flushPendingThink(state.turns).concat([{ kind: 'sys', text }]));
    }
    case 'compaction': {
      const summary = typeof data.summary === 'string' ? data.summary : 'compacted';
      return withTurns(state, closeOpenText(flushPendingThink(state.turns)).concat([{ kind: 'sys', text: '🗜 ' + summary }]));
    }
    case 'agent_switched':
      return withTurns(state, flushPendingThink(state.turns).concat([{ kind: 'sys', text: 'agent → ' + String(data.agent || '') }]));
    case 'model_switched':
      return withTurns(state, flushPendingThink(state.turns).concat([{ kind: 'sys', text: 'model → ' + String(data.model || '') }]));
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
      return withTurns(state, closeOpenText(flushPendingThink(state.turns)).concat([turn]));
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
      return withTurns(
        state,
        settleTerminal(state.turns).concat([{ kind: 'sys', text: '⏹ interrupted' }]),
      );
    case 'transcript_reset':
      // Wire payload is `{}` (runner/event.rs) — the collapsed transcript
      // cannot be rebuilt from the frame. chat.jsx reacts to this event by
      // re-fetching the store snapshot (same path as `done`); here we only
      // close the open text turn so a lost reload still renders sanely.
      // pendingEcho is INTENTIONALLY preserved: the reset fires inside the
      // admitted turn (compound `/act_clear_context <tail>`), and the store
      // snapshot has not recorded the echo yet — the reload re-pushes it
      // (ensurePendingEcho, TUI rebuild_after_reset parity).
      return { ...state, turns: closeOpenText(settleTerminal(state.turns)) };
    case 'done':
      // A pure-text round (or the turn's final Say round) folds its think
      // turns into a call-less step — no think turn survives above the ladder.
      // Turn complete: its echo must never resurface on a later rebuild
      // (TUI parity — a bare /act_clear_context mid-run must not resurrect
      // the previous turn's prompt).
      return {
        ...state,
        status: 'done',
        pendingEcho: null,
        turns: closeOpenText(flushPendingThink(settleTerminal(state.turns))),
      };
    case 'error': {
      // A lag-marked error (api.rs map_broadcast_result) is a consumer
      // re-sync signal, not a run failure: sse.js restarts from the persisted
      // head while the run keeps producing. Folding it terminal would latch
      // status 'error' mid-run — and, since chat.jsx releases busy on terminal
      // errors, would free the composer for a still-running turn.
      if (data && Number.isFinite(data.lag)) {
        return state;
      }
      return {
        ...state,
        status: 'error',
        error: String(data.error || data.message || 'error'),
        pendingEcho: null,
        turns: closeOpenText(flushPendingThink(settleTerminal(state.turns))),
      };
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
/// would have recorded — never the command token itself. `optimistic` marks
/// the turn as a LOCAL prediction: a later steer/queue_consumed frame with
/// the SAME text folds into it (dedup below) instead of rendering twice.
export function withUserTurn(state, text, optimistic) {
  if (!text) {
    return state;
  }
  // Also remember the echo as pendingEcho: a mid-run transcript_reset rebuild
  // must re-push this boundary if the store snapshot lacks it.
  const echoTurn = { kind: 'text', role: 'user', text };
  if (optimistic) {
    echoTurn.optimistic = true;
  }
  return withTurns(
    { ...state, pendingEcho: text },
    closeOpenText(flushPendingThink(settleTerminal(state.turns))).concat([echoTurn]),
  );
}

/// Mirror of the TUI's rebuild_after_reset echo re-push
/// (session_ui/replay.rs): after a mid-run TranscriptReset the transcript is
/// rebuilt from the store snapshot, but the admitted turn's echo (e.g. the
/// tail of a compound `/act_clear_context <tail>`) is recorded only AFTER the
/// reset fired — the snapshot lacks it, and without re-pushing, the running
/// turn loses its user boundary (its steps merge into the previous turn).
/// If `echo` is non-empty and the LAST user text turn in `turns` does not
/// already carry it, append a user echo turn; otherwise return `turns`
/// unchanged (including an empty echo — nothing to anchor).
export function ensurePendingEcho(turns, echo) {
  const text = typeof echo === 'string' ? echo : '';
  if (!text.trim()) {
    return turns;
  }
  const list = Array.isArray(turns) ? turns : [];
  for (let i = list.length - 1; i >= 0; i -= 1) {
    if (list[i] && list[i].kind === 'text' && list[i].role === 'user') {
      return list[i].text === text ? list : list.concat([{ kind: 'text', role: 'user', text }]);
    }
  }
  return list.concat([{ kind: 'text', role: 'user', text }]);
}

/// Resync rebuild (round-2 #5): a reconnecting stream must NOT fold its
/// replay tail into the dirty live state — live frames carry no seq, so the
/// client cannot tell which of them the replay re-delivers, and every frame
/// consumed since the last id'd one would double-fold. Instead the fold state
/// is rebuilt from the store snapshot at a watermark: `headSeq` is the /seq
/// head read BEFORE the snapshot fetch, so (i) everything persisted ≤ head
/// has its message effects in the snapshot and is never replayed, (ii) the
/// replay stream opens after=head and only carries the future tail. The
/// snapshot's `draining` flag closes the finished-while-disconnected gap: a
/// run that ended during the outage has its terminal frame at seq ≤ head
/// (never replayed), so a non-draining rebuild lands terminal `done` and
/// releases busy instead of latching 'streaming' forever. The in-flight
/// turn's partial text truncates at the watermark (its message row lands
/// only at turn end) and converges on the done → snapshot reload. Pure.
export function resyncState({ messages, draining, headSeq, pendingEcho }) {
  const msgs = Array.isArray(messages) ? messages : [];
  const echo = typeof pendingEcho === 'string' ? pendingEcho : null;
  return {
    ...emptyStream(),
    status: draining ? 'streaming' : 'done',
    turns: ensurePendingEcho(turnsFromMessages(msgs), echo),
    usage: usageFromMessages(msgs),
    applySeq: Number.isFinite(headSeq) ? headSeq : null,
    pendingEcho: draining ? echo : null,
  };
}
