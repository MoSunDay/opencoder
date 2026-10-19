const created = 1792300000000;
const plan = {
  schema_version: 7, title: '通用验证计划 · 一段需要完整查看的较长执行计划名称',
  objective: '## 运行目标\n\n完成任务并核对执行结果。\n\n- 保留完整记录\n- 根据结果判断是否达标',
  max_rounds: 5, inputs: { task: '默认输入' },
  layers: [{ layer_id: 'prepare', title: '准备', task: '准备任务', objective: '输入齐全', success_criteria: '检查完成' },
    { layer_id: 'verify', title: '验证', task: '验证任务', objective: '结果准确', success_criteria: '验证通过' }],
  nodes: [{ node_id: 'prepare-task', layer_id: 'prepare', title: '准备任务', objective: '准备输入', capability_id: 'cap' },
    { node_id: 'verify-task', layer_id: 'verify', title: '验证任务', objective: '核对结果', capability_id: 'cap' }],
};
const operations = [1, 2, 3].map((activation) => ({ activation, round: activation === 1 ? 1 : 2, layer: activation === 3 ? 2 : 1,
  node_id: activation === 3 ? 'verify-task' : 'prepare-task', operation_id: `op-${activation}`, execution_id: `operator-${activation}`,
  execution_kind: 'operator', capability_id: 'cap', status: activation === 1 ? 'error' : 'done' }));
const events = operations.flatMap((operation, index) => [
  { ...operation, seq: index * 2 + 1, event_type: 'layer_started', at_ms: created + index * 1000, reason_summary: '## 调度理由\n\n依据完整结果继续执行。',
    reflection: index === 1 ? '根据失败记录重新准备输入。' : null,
    evidence_execution_ids: index ? [operations[index - 1].execution_id] : [], assignments: [{ node_id: operation.node_id, inputs: { task: { kind: 'value', value: '本次任务输入' } } }] },
  { ...operation, seq: index * 2 + 2, event_type: 'milestones_assessed', at_ms: created + index * 1000 + 500,
    reason_summary: '核对任务结果', assessments: { [operation.layer === 1 ? 'prepare' : 'verify']: { met: index !== 0, reason: index === 0 ? '条件未满足' : '验证通过' } } },
]);
const view = { schema_version: 7, plan, layers: [['prepare-task'], ['verify-task']], operations, events, capabilities: [],
  run: { run_id: 'brain-current', phase: 'completed', round: 2, max_rounds: 5, activation: 3, valid_layers: 2, layer: 2, last_event_seq: 6,
    created_at: created, updated_at: created + 3000, summary: '## 已完成\n\n所有阶段已核对。\n\n| 项目 | 结果 |\n| --- | --- |\n| 验证 | 通过 |' } };
function execution(id) {
  return { execution: { id, kind: id.startsWith('brain') ? 'brain' : 'operator', status: 'done', node_id: 'node-fixture', created_at: created },
    request: { kind: id.startsWith('brain') ? 'brain' : 'operator', input: id.startsWith('brain') ? { layered_request: { inputs: { task: '实际输入', enabled: false, count: 0 }, plan } } : { task: '实际任务输入' } },
    result: { scheduler_output: { summary: '执行输出已生成', passed: false, details: { message: '仍需结合层级评估判断是否达标', values: [] } },
      scheduler_artifacts: [{ execution: { id, kind: 'operator' }, step: 'workspace', file: 'report.txt', bytes: 128, sha256: 'a'.repeat(64) }] } };
}
function response(url) {
  const path = url.pathname;
  if (path === '/api/auth/status') return { enabled: false };
  if (path === '/api/me') return { name: 'display-check', role: 'admin' };
  if (path === '/api/brain/library') return { capabilities: [] };
  if (path === '/api/brain/plan-defs') return { plans: [] };
  if (path === '/api/brain/runs') return { runs: url.searchParams.has('cursor_id')
    ? [{ id: 'brain-old', name: '较早的计划', status: 'error', created_at: created - 10000, node_id: 'node-fixture' }]
    : [{ id: 'brain-current', name: plan.title, status: 'done', created_at: created, node_id: 'node-fixture' },
      { id: 'brain-other', name: plan.title, status: 'cancelled', created_at: created - 1000, node_id: 'node-fixture' },
      { id: 'brain-unnamed', status: 'done', created_at: created - 2000, node_id: 'node-fixture' }],
  next_cursor: url.searchParams.has('cursor_id') ? null : { id: 'brain-unnamed', created_at: created - 2000 } };
  if (/^\/api\/brain\/runs\/[^/]+\/layered$/.test(path)) return view;
  if (/^\/api\/executions\/[^/]+$/.test(path)) return execution(path.split('/').at(-1));
  if (path.endsWith('/events-page')) return { head_seq: 0, events: [], more: false, finished: true };
  if (path.endsWith('/messages')) return { messages: [], more: false };
  return undefined;
}
module.exports = { response, plan };
