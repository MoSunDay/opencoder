// Pure Step-ladder reducers shared by snapshot replay and live SSE folding.
// Turn contract (TUI parity, chat_steps.rs + chat_stream.rs): ONE user input
// owns one or MORE pairs of (n Steps + Say). A Say (non-empty assistant
// text) CLOSES the current sub-turn: reasoning/tool turns that arrive after
// it open a FRESH ladder BELOW the Say and never merge into the group above
// it, so one submission alternates [steps, say, steps, say...]. A Step is
// one reasoning run plus every function call before the next reasoning run.

// A Say is non-empty assistant text. An `image:true` marker text turn is
// presentation, NOT a Say (TUI parity: Image blocks never close a turn).
function isSayTurn(turn) {
  return !!(turn && turn.kind === 'text' && turn.role === 'assistant'
    && typeof turn.text === 'string' && turn.text.length > 0 && !turn.image);
}

// The turn floor: where the CURRENT sub-turn's ladder lives. Scanning
// backwards, the LAST Say wins (the floor sits right below it - TUI
// chat_stream.rs advances turn_block_start past every new Say); a user text
// turn caps the walk first when no Say followed the echo; 0 when neither
// exists yet.
function turnFloor(turns) {
  for (let i = turns.length - 1; i >= 0; i -= 1) {
    const turn = turns[i];
    if (isSayTurn(turn) || (turn && turn.kind === 'text' && turn.role === 'user')) {
      return i + 1;
    }
  }
  return 0;
}

// FIRST steps turn at/after the floor (TUI parity: position() over
// blocks[floor..] - the group above a closing Say is out of reach).
function turnStepsIndex(turns) {
  const floor = turnFloor(turns);
  for (let i = floor; i < turns.length; i += 1) {
    const turn = turns[i];
    if (turn && turn.kind === 'steps' && turn.role === 'assistant') {
      return i;
    }
  }
  return -1;
}

// A new ladder inserts AT the floor, pushing later presentation rows down
// (TUI parity: merge_turn_call inserts at turn_block_start, so the settled
// order is always `N Steps` followed by the sub-turn's Say).
function turnInsertIndex(turns) {
  return turnFloor(turns);
}

// Whether a Say sits at/after the floor. Structurally impossible: turnFloor
// returns the index immediately after the LAST Say (a user text turn caps
// the backwards walk the same way), so by construction no Say can exist
// at/after the floor and this is always false. Kept only as defense-in-depth
// for the scoped progress gate; the actual freeze happens in
// settleTurnProgress, on the Say's first chunk.
function turnHasSay(turns) {
  const floor = turnFloor(turns);
  for (let i = floor; i < turns.length; i += 1) {
    if (isSayTurn(turns[i])) {
      return true;
    }
  }
  return false;
}

// Legacy-fold boundary: the last user text turn. Top-level think items only
// exist in old states; the fold recovers every one of them above the user
// echo (Says are transparent, exactly like TUI absorb_pending_thinking).
function lastUserBoundary(turns) {
  for (let i = turns.length - 1; i >= 0; i -= 1) {
    const turn = turns[i];
    if (turn && turn.kind === 'text' && turn.role === 'user') {
      return i;
    }
  }
  return -1;
}

function reasoningStartsStep(steps) {
  const last = steps[steps.length - 1];
  return !last || (Array.isArray(last.calls) && last.calls.length > 0);
}

export function appendThinkDelta(turns, text) {
  const copy = turns.slice();
  const progressActive = !turnHasSay(copy);
  const index = turnStepsIndex(copy);
  if (index < 0) {
    // A fresh ladder can only open BELOW the streaming Say here (the floor
    // sits under the LAST Say) - new ladder activity ends the sub-turn the
    // Say was closing, so its Say-row running hint retires first. Appends
    // into an EXISTING ladder (same sub-turn, no Say yet) never reach this
    // branch and never touch the flag.
    const cleared = clearSayStreaming(copy);
    cleared.splice(turnInsertIndex(cleared), 0, {
      kind: 'steps',
      role: 'assistant',
      progressActive,
      steps: [{ thinking: text, calls: [] }],
    });
    return cleared;
  }
  const group = copy[index];
  const steps = group.steps.slice();
  const last = steps[steps.length - 1];
  if (reasoningStartsStep(steps)) {
    steps.push({ thinking: text, calls: [] });
  } else {
    steps[steps.length - 1] = { ...last, thinking: (last.thinking || '') + text };
  }
  copy[index] = { ...group, progressActive, steps };
  return copy;
}

export function closeOpenText(turns) {
  const last = turns[turns.length - 1];
  if (last && last.kind === 'text' && last.open) {
    const copy = turns.slice();
    copy[copy.length - 1] = { ...last, open: false };
    return copy;
  }
  return turns;
}

// Freeze the progress animation of the FIRST steps group at/after the floor
// - the ladder the incoming Say is about to close - without mutating the
// input array (TUI parity: set_turn_progress(false) fires on the Say's first
// chunk, before the new Say lands, so the next ladder below re-arms fresh).
// ToolEnd intentionally does not call this: progress stays visible through
// the inter-round gap and stops only when Say begins or the run reaches a
// terminal boundary.
export function settleTurnProgress(turns) {
  const index = turnStepsIndex(turns);
  if (index < 0 || turns[index].progressActive === false) {
    return turns;
  }
  const copy = turns.slice();
  copy[index] = { ...copy[index], progressActive: false };
  return copy;
}

// Say-row running ownership (TUI-parity render model): when a Say starts
// streaming it does NOT make "running" vanish - the hint MOVES from the step
// count onto the Say row and stays there until the sub-turn's ladder really
// ends (a fresh ladder opens below the Say, or the run reaches a terminal
// boundary). `sayStreaming` rides on the SAME turn settleTurnProgress
// freezes (the FIRST steps group at/after the floor - the ladder the
// incoming Say closes). Copy-on-write: already marked → same array.
export function markSayStreaming(turns) {
  const index = turnStepsIndex(turns);
  if (index < 0 || turns[index].sayStreaming === true) {
    return turns;
  }
  const copy = turns.slice();
  copy[index] = { ...copy[index], sayStreaming: true };
  return copy;
}

// Retire every Say-row running hint: a terminal boundary or fresh-ladder
// activity ended the sub-turn whose Say was streaming. Only steps turns
// carrying the flag are copied (others keep their object identity); when NO
// turn carries it the input array is returned unchanged.
export function clearSayStreaming(turns) {
  const list = Array.isArray(turns) ? turns : [];
  const flagged = (turn) => !!(turn && turn.kind === 'steps' && turn.role === 'assistant'
    && turn.sayStreaming === true);
  if (!list.some(flagged)) {
    return list;
  }
  const copy = list.slice();
  for (let i = 0; i < copy.length; i += 1) {
    if (flagged(copy[i])) {
      copy[i] = { ...copy[i], sayStreaming: false };
    }
  }
  return copy;
}

// In-place twin of clearSayStreaming for the mutator-style reducers
// (appendStepCall splices the caller-owned array directly).
function clearSayStreamingInPlace(turns) {
  for (let i = 0; i < turns.length; i += 1) {
    const turn = turns[i];
    if (turn && turn.kind === 'steps' && turn.role === 'assistant'
      && turn.sayStreaming === true) {
      turns[i] = { ...turn, sayStreaming: false };
    }
  }
}

// Compatibility fold for old states that still contain top-level think
// items (the live path streams reasoning straight into the ladder). Recovers
// every think turn above the last user echo - Says are transparent, exactly
// like TUI absorb_pending_thinking - so no think item survives outside the
// ladder. Mutates only the caller-owned copy.
export function absorbSegmentThinking(turns) {
  let thinking = '';
  const boundary = lastUserBoundary(turns);
  for (let i = turns.length - 1; i > boundary; i -= 1) {
    const turn = turns[i];
    if (turn && turn.kind === 'think' && turn.role === 'assistant') {
      thinking = (turn.text || '') + thinking;
      turns.splice(i, 1);
    }
  }
  return thinking;
}

// TUI parity with chat_steps.rs::place_thinking_step: file an orphan
// thinking run at a point where no tool call can consume it (run end, or a
// boundary push). Walking backwards, a Say is TRANSPARENT - remembered as
// the fallback insert position, because the run streamed BEFORE that speech;
// a steps turn absorbs the run as a call-less step; any other turn caps the
// walk and a fresh single-step ladder is inserted right after the cap. When
// the walk exhausts, the ladder lands at the last-seen Say index (ABOVE that
// Say), else at the end. Mutates only the caller-owned array.
export function placeThinkingStep(turns, thinking) {
  if (!thinking) {
    return turns;
  }
  let insertAt = turns.length;
  for (let i = turns.length - 1; i >= 0; i -= 1) {
    const turn = turns[i];
    if (isSayTurn(turn)) {
      insertAt = i;
      continue;
    }
    if (turn && turn.kind === 'steps' && turn.role === 'assistant') {
      turns[i] = { ...turn, steps: turn.steps.concat([{ thinking, calls: [] }]) };
      return turns;
    }
    insertAt = i + 1;
    break;
  }
  turns.splice(insertAt, 0, {
    kind: 'steps', role: 'assistant', progressActive: false,
    steps: [{ thinking, calls: [] }],
  });
  return turns;
}

export function flushPendingThink(turns) {
  const copy = turns.slice();
  const thinking = absorbSegmentThinking(copy);
  return placeThinkingStep(copy, thinking);
}

export function appendStepCall(turns, thinking, call, activateProgress = false) {
  const progressAllowed = !turnHasSay(turns);
  const index = turnStepsIndex(turns);
  if (index < 0) {
    // Fresh ladder BELOW the streaming Say (same ownership move as
    // appendThinkDelta): retire the Say-row running hint on the older turns
    // before the new one lands.
    clearSayStreamingInPlace(turns);
    turns.splice(turnInsertIndex(turns), 0, {
      kind: 'steps', role: 'assistant', progressActive: activateProgress && progressAllowed,
      steps: [{ thinking, calls: [call] }],
    });
    return;
  }
  const group = turns[index];
  const steps = group.steps.slice();
  const last = steps[steps.length - 1];
  if (!last || (thinking && reasoningStartsStep(steps))) {
    steps.push({ thinking, calls: [call] });
  } else {
    steps[steps.length - 1] = {
      ...last,
      thinking: thinking ? (last.thinking || '') + thinking : (last.thinking || ''),
      calls: (Array.isArray(last.calls) ? last.calls : []).concat([call]),
    };
  }
  turns[index] = {
    ...group,
    progressActive: progressAllowed && (activateProgress || group.progressActive === true),
    steps,
  };
}

// Snapshot step fold: appended into the ladder the floor-aware helpers
// locate (the FIRST steps turn at/after the floor - below the LAST Say - or
// a fresh group inserted AT the floor), the SAME positioning the live
// reducers use, which is what keeps live SSE and snapshot replay in shape
// parity. A round with no new thinking keeps appending calls to the current
// Step; new thinking starts the next Step.
export function appendStepTurn(turns, thinking, calls) {
  const index = turnStepsIndex(turns);
  if (index >= 0) {
    const group = turns[index];
    const steps = group.steps.slice();
    const last = steps[steps.length - 1];
    if (!thinking && last) {
      steps[steps.length - 1] = {
        ...last,
        calls: (Array.isArray(last.calls) ? last.calls : []).concat(calls),
      };
    } else {
      steps.push({ thinking, calls });
    }
    turns[index] = {
      ...group,
      steps,
    };
    return;
  }
  turns.splice(turnInsertIndex(turns), 0, {
    kind: 'steps', role: 'assistant', progressActive: false,
    steps: [{ thinking, calls }],
  });
}

export function backfillStepsCall(turns, id, apply) {
  for (let i = turns.length - 1; i >= 0; i -= 1) {
    const turn = turns[i];
    if (!turn || turn.kind !== 'steps' || !Array.isArray(turn.steps)) {
      continue;
    }
    for (let s = turn.steps.length - 1; s >= 0; s -= 1) {
      const step = turn.steps[s];
      const calls = (step && Array.isArray(step.calls)) ? step.calls : [];
      for (let c = calls.length - 1; c >= 0; c -= 1) {
        const call = calls[c];
        if (call && call.output === null && (!id || !call.id || call.id === id)) {
          const nextCalls = calls.slice();
          nextCalls[c] = apply(call);
          const nextSteps = turn.steps.slice();
          nextSteps[s] = { ...step, calls: nextCalls };
          turns[i] = { ...turn, steps: nextSteps };
          return true;
        }
      }
    }
  }
  return false;
}

export function backfillBufferedCall(calls, id, apply) {
  if (!Array.isArray(calls)) {
    return null;
  }
  for (let i = calls.length - 1; i >= 0; i -= 1) {
    const call = calls[i];
    if (call && call.output === null && (!id || !call.id || call.id === id)) {
      const next = calls.slice();
      next[i] = apply(call);
      return next;
    }
  }
  return null;
}
