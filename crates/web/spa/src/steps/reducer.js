// Pure Step-ladder reducers shared by snapshot replay and live SSE folding.
// One visible user turn owns one `steps` item regardless of interleaved Say,
// status, task, or image presentation items. A Step is one reasoning run plus
// every function call before the next reasoning run.

function lastUserBoundary(turns) {
  for (let i = turns.length - 1; i >= 0; i -= 1) {
    const turn = turns[i];
    if (turn && turn.kind === 'text' && turn.role === 'user') {
      return i;
    }
  }
  return -1;
}

function turnStepsIndex(turns) {
  const boundary = lastUserBoundary(turns);
  for (let i = boundary + 1; i < turns.length; i += 1) {
    const turn = turns[i];
    if (turn && turn.kind === 'steps' && turn.role === 'assistant') {
      return i;
    }
  }
  return -1;
}

function turnInsertIndex(turns) {
  const boundary = lastUserBoundary(turns);
  for (let i = boundary + 1; i < turns.length; i += 1) {
    const turn = turns[i];
    if (turn && turn.kind === 'text' && turn.role === 'assistant') {
      return i;
    }
  }
  return boundary + 1;
}

function turnHasSay(turns) {
  const boundary = lastUserBoundary(turns);
  for (let i = boundary + 1; i < turns.length; i += 1) {
    const turn = turns[i];
    if (turn && turn.kind === 'text' && turn.role === 'assistant'
      && typeof turn.text === 'string' && turn.text.length > 0) {
      return true;
    }
  }
  return false;
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
    copy.splice(turnInsertIndex(copy), 0, {
      kind: 'steps',
      role: 'assistant',
      progressActive,
      steps: [{ thinking: text, calls: [] }],
    });
    return copy;
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

// Settle the current user turn's progress animation without mutating the
// input array. ToolEnd intentionally does not call this: progress remains
// visible through the inter-round gap and stops only when Say begins or the
// run reaches a terminal boundary.
export function settleTurnProgress(turns) {
  const index = turnStepsIndex(turns);
  if (index < 0 || turns[index].progressActive === false) {
    return turns;
  }
  const copy = turns.slice();
  copy[index] = { ...copy[index], progressActive: false };
  return copy;
}

// Compatibility fold for old states that still contain top-level think
// items. Mutates only the caller-owned copy.
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

export function placeThinkingStep(turns, thinking) {
  if (!thinking) {
    return turns;
  }
  const index = turnStepsIndex(turns);
  if (index >= 0) {
    const group = turns[index];
    turns[index] = {
      ...group,
      steps: group.steps.concat([{ thinking, calls: [] }]),
    };
    return turns;
  }
  turns.splice(turnInsertIndex(turns), 0, {
    kind: 'steps',
    role: 'assistant',
    progressActive: false,
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

// Snapshot messages preserve provider-round boundaries, but those are not
// Step boundaries. A message with no new thinking keeps appending calls to
// the current Step; new thinking starts the next Step.
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
