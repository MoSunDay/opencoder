export const capability = { id: 'coding', kind: 'agent', target: 'Coder', summary: '编码能力', version: '1', input_desc: '任务', output_desc: '变更', definition: {} };
const layer = (id) => ({ layer_id: id, title: id, task: '执行任务', objective: '交付', success_criteria: '通过测试' });
const node = (id, layer_id) => ({ node_id: id, layer_id, title: id, objective: '执行任务', capability_id: 'coding' });
export const version = { id: 'parallel-plan', version: 1, plan: { schema_version: 7, title: '并行计划', objective: '交付', inputs: {}, max_rounds: 5,
  layers: [layer('coding'), layer('testing')], nodes: [node('code-a', 'coding'), node('code-b', 'coding'), node('test-a', 'testing')] } };
