// bubbleItems.js — pure mappers from reduce.js transcript turns to
// @ant-design/x Bubble.List items. Plain JS on purpose (no JSX): the mapping
// rules (stable keys, role naming, usage footer text) are unit-tested in the
// pure-node suite without a DOM.
//
// Turn shapes come from reduce.js: {kind:'text'|'think'|'tool'|'steps'|'sys',
// role:'user'|'assistant', text?, name?, input?, output?, isError?,
// durationMs?, open?, steps?}. X's BubbleContentType accepts AnyObject, so
// items carry the whole turn as `content` and the per-role contentRender in
// transcript.jsx unpacks it.

/// Map a turn to a Bubble.List role key. Text turns split by chat role
/// (user → right-aligned 'user', everything else → 'ai'); think/tool/steps/
/// sys pass through so each keeps its own borderless content renderer.
export function roleOfTurn(turn) {
  const kind = (turn && turn.kind) || 'text';
  if (kind === 'text') {
    return turn && turn.role === 'user' ? 'user' : 'ai';
  }
  return kind;
}

/// Stable item key: kind + position. Position-based on purpose — the reducer
/// may emit several same-kind turns (e.g. consecutive tool rows), so kind
/// alone would collide, and turn objects have no id in the common path. The
/// same turns array always yields the same key sequence across renders.
function turnKey(turn, index) {
  const kind = (turn && turn.kind) || 'text';
  return kind + ':' + index;
}

/// turns → Bubble.List items. Always returns an array; never throws on
/// null/undefined input (an empty transcript renders the hint instead).
export function itemsFromTurns(turns) {
  const list = Array.isArray(turns) ? turns : [];
  return list.map((turn, index) => ({
    key: turnKey(turn, index),
    role: roleOfTurn(turn),
    content: turn,
  }));
}

function finiteOr(value, fallback) {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

/// usage → '▲ in X  ▼ out Y  Σ Z' (+ ' · 上下文 N%' only when the stream
/// carried a context-window figure, capped at 999% like the old footer).
/// Missing fields degrade to 0 placeholders so partial llm_usage frames never
/// render "undefined". Same text format as the pre-migration UsageFooter.
export function usageLine(usage) {
  const u = usage || {};
  const input = finiteOr(u.input, 0);
  const output = finiteOr(u.output, 0);
  const total = finiteOr(u.total, 0);
  let pct = '';
  if (finiteOr(u.contextWindow, 0) > 0) {
    pct = ' · 上下文 ' + Math.min(999, Math.round((total / u.contextWindow) * 100)) + '%';
  }
  return '▲ in ' + input + '  ▼ out ' + output + '  Σ ' + total + pct;
}

/// Empty-transcript gate, equivalent to the old TranscriptView logic: the hint
/// shows only when there are no turns AND no usage chip to show.
export function isEmptyTranscript(turns, usage) {
  const noTurns = !Array.isArray(turns) || turns.length === 0;
  return noTurns && !usage;
}
