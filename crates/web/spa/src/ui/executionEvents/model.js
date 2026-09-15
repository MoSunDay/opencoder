// Pure event-window and display projections, shared by execution types.
export const LOG_LIMIT = 2 * 1024 * 1024;
export const ROW_LIMIT = 1000;
export const initialLogs = () => ({ frames: [], cursor: 0, bytes: 0, trimmed: false });

export function appendLog(state, frame) {
  if (Number.isSafeInteger(frame.seq) && frame.seq <= state.cursor) return state;
  const frames = [...state.frames, frame];
  let bytes = state.bytes + JSON.stringify(frame).length;
  let trimmed = state.trimmed;
  while (frames.length > 1 && (frames.length > ROW_LIMIT || bytes > LOG_LIMIT)) {
    bytes -= JSON.stringify(frames.shift()).length;
    trimmed = true;
  }
  return { frames, bytes, trimmed, cursor: Math.max(state.cursor, frame.seq || 0) };
}

const LABELS = { run_started: '运行开始', step_started: '步骤开始', step_done: '步骤完成', run_finished: '运行结束',
  text_delta: '输出', reasoning_delta: '思考', tool_start: '调用工具', tool_end: '工具结果',
  stdout: 'stdout', stderr: 'stderr', done: '完成', error: '错误', status: '状态' };

export function logEntry(frame) {
  const envelope = frame.data || {};
  const nested = frame.event === 'step_log';
  const event = nested ? (envelope.payload?.event || frame.event) : frame.event;
  const data = nested ? envelope.payload?.data : (envelope.payload ?? envelope);
  let text;
  if (envelope.omitted || data?.omitted) text = '内容较大，可分段查看';
  else if (typeof data === 'string') text = data;
  else if (['tool_start', 'tool_end'].includes(event)) text = `${data?.name || ''}\n${typeof data?.output === 'string' ? data.output : JSON.stringify(data?.input ?? data)}`;
  else text = data?.text ?? data?.error ?? data?.message ?? JSON.stringify(data ?? null);
  return { seq: frame.seq, step: envelope.step || '', event, label: LABELS[event] || event,
    text: String(text), at: envelope.at_ms, marker: envelope.omitted ? envelope : null };
}

export function logRows(frames, step = '', query = '') {
  const needle = query.toLocaleLowerCase();
  const rows = [];
  for (const frame of frames) {
    const entry = logEntry(frame);
    if (step && entry.step !== step) continue;
    const prior = rows.at(-1);
    // Consecutive token/byte fragments read as one line without changing
    // the underlying sequence cursor or hiding interleaved step activity.
    if (prior && prior.step === entry.step && prior.event === entry.event
      && ['text_delta', 'reasoning_delta', 'stdout', 'stderr'].includes(entry.event) && !entry.marker) {
      prior.text += entry.text;
    } else rows.push({ ...entry });
  }
  return needle ? rows.filter((entry) => `${entry.step} ${entry.label} ${entry.text}`.toLocaleLowerCase().includes(needle)) : rows;
}

export function pageFrames(page) {
  return (page.events || []).map((event) => ({ seq: event.seq, event: event.kind || event.event, data: event.data || event.payload || {} }));
}
