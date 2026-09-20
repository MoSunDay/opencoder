// Pure projections for the DAG single-step stream (node-side records).
// wasm steps deliver `step_output` frames ({step, stream, text, at_ms}) plus
// run-session `step_log` mirrors ({kind, step, payload:{event,data}, at_ms});
// agent steps deliver child-session frames that feed reduce.js directly, so
// only the wasm-side log projection lives here.

/// Step kind → 中文 label (worker `spec_step_kind` vocabulary: wasm | agent).
export const STEP_KIND_LABEL = { wasm: 'Wasm 步骤', agent: 'Agent 步骤', dynamic: '动态步骤' };

export function isAgentKind(kind) {
  return kind === 'agent';
}

const OUTPUT_LABEL = { stdout: 'stdout', stderr: 'stderr', text_delta: '输出' };

/// Normalize one frame into {stream, text}, or null when it carries no
/// output text. Accepts flat `step_output` (node-side wasm capture) and the
/// nested `step_log` mirror (run-session shape) alike; every other kind is
/// ignored so agent-session frames never leak into the log view.
function outputOf(frame) {
  const envelope = frame.data || {};
  if (frame.event === 'step_output') {
    const stream = envelope.stream === 'stderr' ? 'stderr' : 'stdout';
    return { stream, text: String(envelope.text ?? '') };
  }
  if (frame.event === 'step_log') {
    const event = envelope.payload?.event;
    if (!['stdout', 'stderr', 'text_delta'].includes(event)) return null;
    const data = envelope.payload?.data;
    const text = typeof data === 'string' ? data : String(data?.text ?? '');
    return { stream: event, text };
  }
  return null;
}

/// Project frames into wasm log rows {seq, at, stream, label, text}.
/// Adjacent same-stream fragments merge into one row (logRows parity);
/// stdout/stderr never merge into each other. `query` is a case-insensitive
/// substring filter over label + text.
export function outputRows(frames, query = '') {
  const needle = String(query ?? '').toLocaleLowerCase();
  const rows = [];
  for (const frame of frames) {
    const output = outputOf(frame);
    if (!output) continue;
    const prior = rows.at(-1);
    if (prior && prior.stream === output.stream) {
      prior.text += output.text;
      continue;
    }
    rows.push({
      seq: frame.seq,
      at: (frame.data || {}).at_ms,
      stream: output.stream,
      label: OUTPUT_LABEL[output.stream] || output.stream,
      text: output.text,
    });
  }
  return needle
    ? rows.filter((row) => `${row.label} ${row.text}`.toLocaleLowerCase().includes(needle))
    : rows;
}

/// The last `step_finished` frame's data ({status, error, started_at_ms,
/// finished_at_ms}), or null while the step has not reached a terminal state.
export function finishedOf(frames) {
  for (let i = frames.length - 1; i >= 0; i -= 1) {
    if (frames[i] && frames[i].event === 'step_finished') return frames[i].data || null;
  }
  return null;
}
