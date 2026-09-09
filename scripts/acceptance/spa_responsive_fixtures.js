// scripts/acceptance/spa_responsive_fixtures.js
//
// Fixture API for scripts/acceptance/spa_responsive.js. Response shapes mirror
// what each panel actually reads (checked against src/*.jsx and the panels' own
// dom tests) so tables render real rows — an empty table cannot overflow and
// would let the gate pass vacuously. Only GETs are modelled: the gate is
// read-only and answers anything else 405.

const L = (n, f) => Array.from({ length: n }, (_, i) => f(i));
const NOW = 1770000000000;

const NODE = (i) => ({
  id: `node-${i}`, name: `node-${i}`, host: '10.0.0.7', status: i ? 'idle' : 'busy',
  state: 'online', phase: 'running', mode: 'auto', os: 'linux', arch: 'amd64',
  cpu: 3.2, cpu_limit: 8, mem_mb: 1024, mem_limit_mb: 8192, disk_mb: 2048,
  disk_limit_mb: 20480, gpu: 0, gpu_limit: 0, last_seen: NOW, last_heartbeat: NOW,
  labels: { region: 'cn-north', tier: 'gpu' },
  resources: { cpu_pct: 12, mem_mb: 512, mem_total_mb: 8192, disk_free_mb: 2048, load1: 0.4 },
  capacity: { cpu: 8, mem_mb: 8192, disk_mb: 20480 },
  runtime: { docker: true, cuda: false, version: '0.4.0' },
});

const EXEC = (i) => ({
  id: `exe-${i}`, execution_id: `exe-${i}`, run_id: `run-${i}`, node_id: `node-${i % 3}`,
  agent_id: `agent-${i % 2}`, session_id: 'ses-1', kind: ['agent', 'dag', 'todo'][i % 3],
  state: ['running', 'succeeded', 'failed'][i % 3],
  status: ['running', 'succeeded', 'failed'][i % 3],
  progress: i % 3 === 0 ? 42 : 100, message: '', error: '',
  created_at: NOW + i, updated_at: NOW + 1000 + i, finished_at: NOW + 2000,
});

const AGENT = (i) => ({ id: `agent-${i}`, name: `智能体 ${i}`, description: '通用编码 agent',
  mode: 'auto', model: 'gpt-4o-mini', prompt: 'p', tools: ['bash', 'edit'], envs: {},
  max_turns: 8, version: 'v1' });

const TEAM = (i) => ({ name: `team-${i}`, description: '值班团队', policy: 'round-robin',
  members: ['node-0', 'node-1'], leader: 'agent-0' });

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
    steps: [{ id: 's1', kind: 'agent', prompt: 'p' }, { id: 's2', kind: 'shell', command: 'ls' }] } });

const DAG_RUN = (i) => ({ id: `drun-${i}`, def_id: `dag-${i}`, name: `运行 ${i}`, node_id: `node-${i % 3}`,
  status: ['running', 'succeeded', 'failed'][i % 3], state: ['running', 'succeeded', 'failed'][i % 3],
  progress: 50, trigger: 'manual', created_at: NOW + i, started_at: NOW + i,
  finished_at: NOW + 1000, error: '', spec: DAG_DEF(i).spec });

const TODO = (id, title, status, ms) => ({ id, milestone_id: ms, title, status,
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
const ABSENT = ['/api/models', '/api/agents/nfs', '/api/project/todos/todo-0/runs',
  '/api/sessions/ses-1/questions', '/api/sessions/ses-1/inputs'];

const FIXTURES = {
  '/api/health': { ok: true, version: '0.0.0-fixture' },
  // main.jsx re-probes /api/me per token; `name` unlocks IdentityBadge and
  // role=admin unlocks the admin entries (store.js setIdentity).
  '/api/me': { name: 'root', role: 'admin', username: 'root' },
  '/api/users': { users: [{ name: 'root', role: 'admin', created_at: NOW, last_seen: NOW }] },
  '/api/nodes': { nodes: L(3, NODE) },
  '/api/nodes/node-0': NODE(0),
  '/api/nodes/node-0/dialogs': { dialogs: [] },
  '/api/executions': { executions: L(4, EXEC), next_cursor: null },
  '/api/executions/exe-0': EXEC(0),
  '/api/teams': { teams: L(2, TEAM) },
  '/api/agents': { agents: L(2, AGENT), active: 'agent-0' },
  '/api/agents/agent-0': AGENT(0),
  '/api/agents/resources/prompts': { resources: [{ name: 'p1', kind: 'prompt', path: '/a/p1.md' }] },
  '/api/agents/resources/skills': { resources: [{ name: 's1', kind: 'skill', path: '/a/s1' }] },
  '/api/agents/resources/tools': { resources: [{ name: 't1', kind: 'tool', path: '/a/t1' }] },
  '/api/agents/resources/memory': { resources: [{ name: 'm1', kind: 'memory', path: '/a/m1' }] },
  // harness/runners.jsx renders entry.settings.workdir and harness/management.jsx
  // reads data.harnesses.find(name === 'codex').settings — both would throw.
  '/api/runners': { items: [{ name: 'runner-0', settings: { command: ['codex', 'exec'],
    workdir: '/srv/work', parent_unit: null, files: {}, envs: { RUST_LOG: 'info' } } }] },
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
  '/api/brain/graph': { nodes: [], edges: [] },
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
