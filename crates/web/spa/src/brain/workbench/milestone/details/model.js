export function roundRows(history) {
  const rounds = new Map();
  for (const visit of history) {
    if (!rounds.has(visit.round)) rounds.set(visit.round, []);
    rounds.get(visit.round).push(visit);
  }
  return [...rounds].sort(([a], [b]) => b - a).map(([round, visits]) => ({
    round, visits, operations: visits.flatMap((visit) => visit.operations),
    layerCount: new Set(visits.map((visit) => visit.layer)).size,
  }));
}

export function assessmentFor(view, layer, activation) {
  const events = view.events || [];
  const assessment = [...events].reverse().find((event) => event.activation === activation && event.assessments?.[layer.layer_id])?.assessments[layer.layer_id];
  if (!assessment) return { label: '尚未评估', color: 'default' };
  if (!assessment.met) return { label: '未达标', color: 'error', reason: assessment.reason };
  const evaluatedLayer = view.plan.layers.findIndex((item) => item.layer_id === layer.layer_id) + 1;
  const invalidated = events.some((event) => event.event_type === 'layer_started' && event.activation > activation && event.layer <= evaluatedLayer);
  return { label: invalidated ? '历史达标，返工后已失效' : '已达标', color: invalidated ? 'default' : 'success', reason: assessment.reason };
}

export const EVENT_LABELS = {
  run_created: '创建运行', layer_started: '派发任务', milestones_assessed: '评估达成情况',
  human_input: '人工补充信息', guidance_processed: '处理人工引导', operation_terminal: '任务执行结束',
  run_completed: '运行完成', run_failed: '运行失败', run_cancelled: '取消运行', run_paused: '暂停调度',
  run_resumed: '恢复调度', decision_blocked: '调度阻塞', operation_admitted: '任务已受理',
  cancel_requested: '已请求取消任务', round_budget_updated: '调整轮次预算', layer_barrier_reached: '本层执行结束',
};
