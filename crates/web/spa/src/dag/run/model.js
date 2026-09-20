import { frameToEvent } from '../../dagProjection.js';

export const isActive = (status) => ['pending', 'running', 'cancelling'].includes(status);

export function checkedSnapshot(value) {
  if (!value || !Number.isSafeInteger(value.head_seq) || value.head_seq < 0
    || typeof value.execution_status !== 'string' || !Array.isArray(value.steps)) {
    throw new Error('节点未返回有效的 DAG 结果快照');
  }
  return value;
}

// One record per step; log frames never grow or repaint the graph state.
export function applyDagFrame(snapshot, frame) {
  const event = frameToEvent(frame);
  if (!event || !snapshot || !Number.isSafeInteger(frame.seq) || frame.seq <= snapshot.head_seq) return snapshot;
  const next = { ...snapshot, head_seq: frame.seq };
  if (event.kind === 'run_finished') {
    return { ...next, execution_status: event.payload.status, execution_error: event.payload.error || '' };
  }
  if (event.kind === 'run_started') return { ...next, execution_status: 'running', execution_error: '' };
  next.steps = snapshot.steps.map((step) => {
    if (step.name !== event.step) return step;
    if (event.kind === 'step_progress') {
      if (event.at_ms < (step.instances_at_ms || 0)) return step;
      return { ...step, instances: event.payload.instances, instances_at_ms: event.at_ms };
    }
    // A receipt may land before its queued lifecycle events.
    if (event.at_ms < (step.at_ms || 0) || (event.kind === 'step_started'
      && event.at_ms === step.at_ms && ['done', 'error', 'cancelled'].includes(step.status))) return step;
    return { ...step, seq: frame.seq, at_ms: event.at_ms,
      status: event.kind === 'step_started' ? 'running' : event.payload.ok === false ? 'error' : 'done',
      error: event.payload.error || null };
  });
  return next;
}
