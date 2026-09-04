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

function isAssistantTurnPart(turn) {
  if (!turn || turn.role !== 'assistant') {
    return false;
  }
  return turn.kind === 'steps' || turn.kind === 'text' || turn.kind === 'think';
}

/// A run part closes the bubble only when it is a Say - non-empty assistant
/// text without the `image` marker (presentation rows never close a Turn).
function isSayPart(part) {
  return !!(part && part.kind === 'text' && part.role === 'assistant'
    && typeof part.text === 'string' && part.text.length > 0 && !part.image);
}

/// Reduced transcript segments → Bubble.List items. A maximal adjacent run of
/// assistant parts containing a steps ladder is one assistant Turn item, and
/// the run ENDS at its Say: the closing speech is the bubble's tail (plus any
/// preceding think/empty parts), so under the pairing contract - one user
/// input alternates [steps, say, steps, say...] - a steps part AFTER a Say
/// starts the NEXT item with its own step count and its own Say. Multiple
/// adjacent step segments before the Say still merge into that single count.
/// Ordinary text-only replies retain the existing ai-item shape. Always
/// returns an array and never throws on null/undefined.
export function itemsFromTurns(turns) {
  const list = Array.isArray(turns) ? turns : [];
  const items = [];
  for (let index = 0; index < list.length;) {
    const turn = list[index];
    if (!isAssistantTurnPart(turn)) {
      items.push({ key: turnKey(turn, index), role: roleOfTurn(turn), content: turn });
      index += 1;
      continue;
    }
    let end = index;
    let hasSteps = false;
    const parts = [];
    while (end < list.length) {
      const part = list[end];
      if (isAssistantTurnPart(part)) {
        parts.push(part);
        end += 1;
        hasSteps ||= part.kind === 'steps';
        // A Say ends the run INCLUSIVELY - the speech is the Turn's tail,
        // never a middle part (image markers and empty texts do not close).
        if (isSayPart(part)) {
          break;
        }
      } else if (part && part.kind === 'sys' && parts.length > 0) {
        // A sys status row inside an open run is presentation (the retry
        // badge), not a Turn boundary - absorb it so the run's
        // [steps ... say] stays ONE bubble instead of splitting in two.
        parts.push(part);
        end += 1;
      } else {
        break;
      }
    }
    if (hasSteps) {
      const stepParts = parts.filter((part) => part.kind === 'steps');
      const steps = stepParts.flatMap((part) => (
        Array.isArray(part.steps) ? part.steps : []
      ));
      const say = parts.filter((part) => part.kind !== 'steps');
      const hasSay = say.some((part) => (
        part.kind === 'text' && typeof part.text === 'string' && part.text.length > 0
      ));
      const progressActive = hasSay
        ? false
        : (stepParts.some((part) => part.progressActive === true)
          ? true
          : (stepParts.every((part) => part.progressActive === false) ? false : undefined));
      // Say-row running ownership: a sayStreaming ladder (its Say is
      // streaming) keeps the Turn's "running" hint alive ON THE SAY ROW even
      // though progressActive froze — until fresh ladder activity or a
      // terminal boundary retires the flag. Stays false without any flag
      // (snapshot replay turns never carry one).
      const sayActive = stepParts.some((part) => part.sayStreaming === true);
      items.push({
        key: 'assistant-turn:' + index,
        role: 'assistantTurn',
        content: {
          kind: 'assistant_turn', role: 'assistant', steps, say, progressActive, sayActive,
        },
      });
    } else {
      parts.forEach((part, offset) => {
        const partIndex = index + offset;
        items.push({ key: turnKey(part, partIndex), role: roleOfTurn(part), content: part });
      });
    }
    index = end;
  }
  return items;
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
