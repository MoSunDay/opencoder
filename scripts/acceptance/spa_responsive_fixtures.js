// scripts/acceptance/spa_responsive_fixtures.js
//
// Fixture API for scripts/acceptance/spa_responsive.js. Response shapes mirror
// what each panel actually reads (checked against src/*.jsx and the panels' own
// dom tests) so tables render real rows — an empty table cannot overflow and
// would let the gate pass vacuously. Only GETs are modelled: the gate is
// read-only and answers anything else 405.

const L = (n, f) => Array.from({ length: n }, (_, i) => f(i));
const NOW = 1770000000000;
const { ONTOLOGY_FIXTURES } = require('./spa_responsive_ontology');

const NODE = (i) => ({
  id: `node-${i}`, name: `node-${i}`, online: true,
  kinds: ['agent', 'operator', 'dag', 'team', 'todos', 'brain'],
  capabilities: ['dag_container_v1', 'dag_dynamic_v1'], maintenance_agent_id: `maintenance-${i}`,
  snapshot: { ready: true, resource_error: null, cpu_capacity: 8, active_runs: 1,
    max_runs: 4, pending_runs: 0, queue_order: 'fifo', active_agent_loops: 1 },
});

const EXEC = (i) => ({
  id: `exe-${i}`, name: `执行 ${i}`, node_id: `node-${i % 3}`,
  kind: ['agent', 'dag', 'todos'][i % 3], status: ['idle', 'done', 'error'][i % 3], created_at: NOW + i,
});

const AGENT = (i) => ({ id: `agent-${i}`, name: `智能体 ${i}`, description: '通用编码 agent',
  mode: 'auto', model: 'gpt-4o-mini', prompt: 'p', tools: ['bash', 'edit'], envs: {},
  max_turns: 8, version: 'v1' });

const TEAM = (i) => ({ name: `team-${i}`, description: '值班团队', policy: 'round-robin',
  members: [{ agent: 'act' }, { agent: 'plan' }], captain: 'plan' });

const WORKFLOW = (i) => ({ id: `wf-${i}`, todo_id: `todo-${i}`, status: ['running', 'done', 'failed'][i % 3],
  attempt: i % 2, active_session_id: 'ses-1', node_id: 'node-0', updated_at: NOW + i,
  created_at: NOW, events: 4 });

const TEMPLATE = (i) => ({ name: `模板-${i}`, description: '日常巡检模板', tools: ['/agent/tools/v3/git'],
  env_vars: { PATH: '/usr/bin' }, prompt: 'p' });

// brainPanel.jsx reads row.capability.* (rowKey = row.capability.id).
const CAP = (i) => ({ capability: { id: `cap-${i}`, capability_type: ['goal', 'task', 'tool'][i % 3],
  summary: '解析依赖图并给出构建顺序', input_desc: 'crate 列表', output_desc: '依赖 DAG',
  updated_at: NOW + i }, eng_inputs: [{ content: 'opencoder' }] });

const DAG_DEF = (i) => ({ id: `dag-${i}`, name: `工作流定义 ${i}`, version: 1, updated_at: NOW + i,
  updated_by: 'root', spec: { name: `工作流定义 ${i}`, description: '发布前回归',
    steps: [{ name: 'analyze', kind: { type: 'agent', prompt: 'p' } },
      { name: 'build', depends_on: ['analyze'], kind: { type: 'binary', resource: 'build@v1', args: [] } }] } });

const DAG_RUN = (i) => ({ id: `drun-${i}`, def_id: `dag-${i}`, name: `运行 ${i}`, node_id: `node-${i % 3}`,
  status: ['running', 'succeeded', 'failed'][i % 3], state: ['running', 'succeeded', 'failed'][i % 3],
  progress: 50, trigger: 'manual', created_at: NOW + i, started_at: NOW + i,
  finished_at: NOW + 1000, error: '', spec: DAG_DEF(i).spec });

const TODO = (id, title, status, ms) => ({ id, initiative_id: ms, title, status,
  created_at: NOW, updated_at: NOW, assignee: 'agent-0', detail_error: null });

const GOAL = (id, title) => ({
  id, title, status: 'active',
  milestones: [
    { id: `${id}-m1`, goal_id: id, title: 'M1 冲刺', status: 'in_progress',
      todos: [TODO(`${id}-t1`, '写发布说明', 'done', `${id}-m1`), TODO(`${id}-t2`, '回归测试', 'failed', `${id}-m1`)] },
    { id: `${id}-m2`, goal_id: id, title: 'M2 打磨', status: 'planned',
      todos: [TODO(`${id}-t3`, '性能压测', 'planned', `${id}-m2`)] },
  ],
});

// Routes the SPA may poll for data this gate has not modelled. They must stay
// 404 so panels keep their empty state instead of rendering invented rows.
const ABSENT = ['/api/models', '/api/project/todos/todo-0/runs',
  '/api/sessions/ses-1/questions', '/api/sessions/ses-1/inputs'];

const FIXTURES = {
  ...ONTOLOGY_FIXTURES,
  '/api/health': { ok: true, version: '0.0.0-fixture' },
  // main.jsx re-probes /api/me per token; `name` unlocks IdentityBadge and
  // role=admin unlocks the admin entries (store.js setIdentity).
  '/api/me': { name: 'root', role: 'admin', username: 'root' },
  '/api/agents/nfs': { status: { running: true, host: '127.0.0.1', port: 2049, read_only: true, export_root: '/fixture/agents' } },
  '/api/dag/binaries/nfs': { root: '/fixture/binaries', status: { running: true, host: '127.0.0.1', port: 2050, read_only: true, export_root: '/fixture/binaries' } },
  '/api/dag/workspace/nfs': { root: '/fixture/source', status: { running: true, host: '127.0.0.1', port: 2051, read_only: true, export_root: '/fixture/source' } },
  '/api/dag/binaries': { pools: [{ name: 'tool', description: 'Linux executable', current: 2, current_version: { version: 2, size_bytes: 1024, sha256: 'a'.repeat(64) } }] },
  '/api/dag/binaries/tool': { name: 'tool', current: 2, history: [{ version: 1, size_bytes: 1024, sha256: 'b'.repeat(64) }, { version: 2, size_bytes: 1024, sha256: 'a'.repeat(64) }] },
  '/api/admin/release': { enabled: false },
  '/api/users': { users: [{ name: 'root', role: 'admin', created_at: NOW, last_seen: NOW }] },
  '/api/nodes': { nodes: L(3, NODE) },
  '/api/nodes/node-0': NODE(0),
  '/api/nodes/node-0/dialogs': { dialogs: [] },
  '/api/executions': { executions: L(4, EXEC), next_cursor: null },
  '/api/executions/exe-0': { execution: EXEC(0), request: { kind: 'agent', target: 'act', input: {} }, result: null, annotations: {} },
  '/api/executions/exe-0/messages': { chunks: [], next_cursor: null, more: false },
  '/api/nodes/node-0/scheduling': { max_runs: 4, queue_order: 'fifo', workdir_supported: false },
  '/api/schedules': { schedules: [{ id: 'fixture-schedule', kind: 'dag', target: 'dag-0', cron: '0 3 * * *', timezone: '+08:00', enabled: false, overlap: 'skip', params: { args: [] } }] },
  '/api/teams': { teams: L(2, TEAM) },
  '/api/agents': { agents: L(2, AGENT), active: 'agent-0' },
  '/api/agents/agent-0': AGENT(0),
  '/api/agents/resources/prompts': { resources: [{ name: 'p1', kind: 'prompt', path: '/a/p1.md' }] },
  '/api/agents/resources/skills': { resources: [{ name: 's1', kind: 'skill', path: '/a/s1' }] },
  '/api/agents/resources/tools': { resources: [{ name: 't1', kind: 'tool', path: '/a/t1' }] },
  '/api/agents/resources/memory': { resources: [{ name: 'm1', kind: 'memory', path: '/a/m1' }] },
  // harness/management.jsx reads data.harnesses.find(name === 'codex').settings.
  '/api/harnesses': { harnesses: [{ name: 'codex', revision: 3, settings: { executable: '/usr/bin/codex',
    model: 'gpt-4o-mini', reasoning_effort: 'medium', sandbox_mode: 'read-only',
    approval_policy: 'never', envs: {}, auth_slot: null } }],
    profiles: [{ name: 'default', model: 'gpt-4o-mini' }] },
  '/api/skills': { skills: [{ name: 'git', description: 'git 操作', disabled: false }] },
  '/api/todo/envs': { envs: [{ name: 'demo', description: '视频工具链',
    tools: ['/agent/tools/v3/ffmpeg'], env_vars: { FFMPEG_PATH: '/usr/bin/ffmpeg' } }] },
  '/api/todo/tools': { tools: [{ ref: '/agent/tools/v3/ffmpeg', source: 'share' },
    { ref: '/agent/tools/v2/git', source: 'importable', agent: 'agent-1', version: 'v2', tool: 'git' }] },
  '/api/todo/templates': { templates: L(2, TEMPLATE) },
  '/api/todo/templates/模板-0': TEMPLATE(0),
  '/api/todo/workflows': { workflows: L(3, WORKFLOW) },
  '/api/brain/capabilities': { capabilities: L(3, CAP) },
  '/api/brain/agents': { agents: [{ agent: 'default', capabilities: [{ summary: '代码审查' }] }] },
  '/api/brain/graph': { nodes: [], edges: [] },
  '/api/brain/plan-defs': { plans: [] },
  '/api/brain/runs': { runs: [] },
  '/api/brain/library': { capabilities: [] },
  // dag/defsTab.jsx and dag/runsTable.jsx both require a BARE array.
  '/api/dag/defs': L(2, DAG_DEF),
  '/api/dag/runs': L(3, DAG_RUN),
  '/api/dag/runs/drun-0': DAG_RUN(0),
  '/api/project/overview': { goals: [GOAL('g1', '发布 1.0'), GOAL('g2', '站点改版')],
    backlog: [TODO('b1', '整理巡检脚本', 'planned', null)], standalone_milestones: [] },
};

// Endpoints that require the bearer token (src/admin/*).
const GUARDED = ['/api/users'];

module.exports = { FIXTURES, ABSENT, GUARDED };
