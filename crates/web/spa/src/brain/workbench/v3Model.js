export const V3_PHASES = {
  ready: '等待调度', deciding: '判断中', waiting: '等待执行', paused: '已暂停',
  blocked: '已阻塞', completed: '已完成', failed: '失败', cancelled: '已取消',
};

export const V3_STATUS = {
  creating: '创建中', running: '执行中', done: '执行结束', error: '失败', cancelled: '已取消',
};

export const V3_COLORS = {
  ready: 'cyan', deciding: 'purple', waiting: 'blue', paused: 'gold', blocked: 'red',
  completed: 'green', failed: 'red', cancelled: 'default', creating: 'gold', running: 'blue',
  done: 'green', error: 'red',
};

export function phaseOf(view) {
  return view?.run?.phase || view?.phase || 'ready';
}

export function terminalV3(view) {
  return ['completed', 'failed', 'cancelled'].includes(phaseOf(view));
}

export function roundsOf(view) {
  return (view?.rounds || []).map((round) => ({
    ...round,
    operations: [...(round.operations || [])].sort((a, b) => a.execution_id.localeCompare(b.execution_id)),
  }));
}

export function currentRound(view) {
  const round = Number(view?.run?.round || 0);
  return round || roundsOf(view).at(-1)?.round || 0;
}

export function capabilityFor(view, operation) {
  const found = (view?.capabilities || []).find((capability) => (
    capability.capability_id === operation.capability_id
  ));
  return found || {
    capability_id: operation.capability_id,
    kind: operation.execution_kind,
    target: '索引中未保存目标',
    version: 'unknown',
  };
}

export function roundLabel(round) {
  return round === 0 ? '尚未调度' : `第 ${round} 轮`;
}

export function normalizeEvent(value) {
  if (!value) return null;
  const event = value.event_type || value.kind || value.event || 'scheduler_event';
  return {
    seq: value.seq,
    event,
    data: value.data || value.payload || value,
    ts: value.at_ms || value.ts,
  };
}
