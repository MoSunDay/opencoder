// @vitest-environment jsdom
// TODO tab executor-dimension DOM smoke (P4): the create modal's 执行器
// select offers the four kinds (agent/team/dag/brain) with kind-conditional
// ref/spec fields, the table's 执行器 column tags every row kind (+ref
// text), the built POST body carries the executor slice (and an invalid
// spec JSON never leaves the browser), and the runs drawer shows the
// RESOLVED executor kind with brain provenance / output refs. api calls are
// routed by URL (same contract as project.dom.test). The pure helpers
// (executorBody / executorRefText / EXECUTOR_OPTIONS) are asserted directly
// — mirror of the exported-helper style used across the suite.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';

const { apiGetMock, apiPostMock } = vi.hoisted(() => ({ apiGetMock: vi.fn(), apiPostMock: vi.fn() }));
vi.mock('../api.js', () => ({ apiGet: apiGetMock, apiPost: apiPostMock, apiPatch: vi.fn(), apiDel: vi.fn() }));
vi.mock('../fleet/detail.jsx', () => ({ ExecutionDetail: ({ id }) => <div>execution-detail:{id}</div> }));

import '../test/setup-dom.js';
import { TodosTab, EXECUTOR_OPTIONS, executorBody, executorRefText } from './todosTab.jsx';
import { TodoDrawer } from './todoDrawer.jsx';

const T0 = 1700000000000;

const overviewFixture = () => ({
  goals: [],
  backlog: [
    {
      id: 't1', milestone_id: null, title: '单跑', draft: 'd', plan_md: null, status: 'draft',
      agent: 'explore', executor_kind: 'agent', created_at: T0, updated_at: T0,
    },
    {
      id: 't2', milestone_id: null, title: '团队跑', draft: 'd', plan_md: '# p', status: 'planned',
      agent: 'act', executor_kind: 'team', executor_ref: 'crew-x', created_at: T0, updated_at: T0,
    },
  ],
});

const runsFixture = () => ({
  runs: [
    {
      id: 'r2', todo_id: 't2', kind: 'execute', version: 2, plan_md: '# p', output_md: 'ok',
      agent: 'act', session_id: 'sess-1', status: 'done', started_at: T0, finished_at: T0,
      created_at: T0, executor_kind: 'dag', capability_id: 'cap-77', plan_id: 'bplan-1234567890',
      output_ref: '/workflow/prun-x1/build-artifacts',
    },
    {
      id: 'r1', todo_id: 't2', kind: 'plan', version: 1, plan_md: null, output_md: null,
      agent: 'act', session_id: null, status: 'done', started_at: T0, finished_at: T0,
      created_at: T0, executor_kind: 'agent',
    },
  ],
});

beforeEach(() => {
  apiGetMock.mockReset().mockImplementation((path) => {
    if (path === '/api/project/todos/t2/runs') {
      return Promise.resolve(runsFixture());
    }
    return Promise.resolve(overviewFixture());
  });
  apiPostMock.mockReset().mockResolvedValue({ id: 'pt-new' });
});

/// antd Tabs/Modal/rc-motion settle — one macrotask keeps every query on the
/// live DOM generation (same contract as project.dom.test).
const settle = () => act(async () => {
  await new Promise((r) => setTimeout(r, 20));
});

/// antd 6 Button 对两字中文自动插空格，按 textContent 去空白匹配（same
/// 口径 as project.dom.test）。
const findButton = (txt, root) => [...(root || document).querySelectorAll('button')]
  .find((b) => (b.textContent || '').replace(/\s+/g, '') === txt);

const mountTab = () => render(
  <TodosTab overview={overviewFixture()} refresh={() => {}} openTodo={() => {}} onNotice={() => {}} />,
);

const openCreate = async () => {
  fireEvent.click(screen.getByText('新建 TODO'));
  await settle();
};

/// 打开执行器 Select（交互面是 .ant-select 根）并在浮层里点 `label`；
/// options portal 到 document.body（same pattern as agentsConfig test）。
const pickExecutor = async (label) => {
  const select = screen.getByLabelText('executor_kind').closest('.ant-select');
  await act(async () => {
    fireEvent.mouseDown(select);
  });
  const option = await waitFor(() => {
    const hit = [...document.querySelectorAll('.ant-select-item-option')]
      .find((o) => o.getAttribute('title') === label || o.textContent === label);
    expect(hit, `executor option ${label}`).toBeTruthy();
    return hit;
  });
  await act(async () => {
    fireEvent.click(option);
  });
  await settle();
};

describe('TodosTab executor column', () => {
  it('tags every row with its executor kind (+ ref/agent secondary text)', async () => {
    mountTab();
    const agentRow = (await screen.findByText('单跑')).closest('tr');
    expect(agentRow && within(agentRow).getByText('单Agent')).toBeTruthy();
    expect(agentRow && within(agentRow).getByText('explore')).toBeTruthy();
    const teamRow = screen.getByText('团队跑').closest('tr');
    expect(teamRow && within(teamRow).getByText('团队')).toBeTruthy();
    expect(teamRow && within(teamRow).getByText('crew-x')).toBeTruthy();
  });
});

describe('TodosTab create modal', () => {
  it('offers the four executor kinds; conditional fields follow the pick', async () => {
    mountTab();
    await openCreate();
    expect(EXECUTOR_OPTIONS.map((o) => o.value)).toEqual(['agent', 'team', 'dag', 'brain']);
    expect(screen.getByLabelText('executor_kind')).toBeTruthy();
    // agent default: the agent input, no ref/spec fields.
    expect(screen.getByPlaceholderText('act')).toBeTruthy();
    expect(screen.queryByLabelText('executor_spec')).toBeNull();
    await pickExecutor('团队');
    expect(screen.queryByPlaceholderText('act')).toBeNull();
    expect(screen.getByPlaceholderText('可留空')).toBeTruthy();
    expect(screen.getByLabelText('executor_spec')).toBeTruthy();
  });

  it('submits the executor slice with the POST body (no agent key for team)', async () => {
    mountTab();
    await openCreate();
    fireEvent.change(screen.getByPlaceholderText('要完成的一件事'), { target: { value: '团队活' } });
    await pickExecutor('团队');
    fireEvent.change(screen.getByPlaceholderText('可留空'), { target: { value: '  crew-x  ' } });
    fireEvent.change(screen.getByLabelText('executor_spec'), {
      target: { value: '{"name":"c","captain":{"node_id":"act","name":"L"}}' },
    });
    fireEvent.click(findButton('创建'));
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith('/api/project/todos', expect.objectContaining({
        title: '团队活',
        executor_kind: 'team',
        executor_ref: 'crew-x',
        executor_spec: '{"name":"c","captain":{"node_id":"act","name":"L"}}',
      }));
    });
    const body = apiPostMock.mock.calls[0][1];
    expect(body.agent).toBeUndefined();
  });

  it('rejects an invalid spec JSON locally without POSTing', async () => {
    mountTab();
    await openCreate();
    fireEvent.change(screen.getByPlaceholderText('要完成的一件事'), { target: { value: 'x' } });
    await pickExecutor('DAG');
    fireEvent.change(screen.getByLabelText('executor_spec'), { target: { value: 'not json' } });
    fireEvent.click(findButton('创建'));
    await settle();
    expect(apiPostMock).not.toHaveBeenCalled();
  });
});

describe('TodoDrawer run rows', () => {
  it('show the resolved executor kind, brain provenance and output_ref', async () => {
    render(
      <TodoDrawer
        todoId="t2"
        overview={overviewFixture()}
        refresh={() => {}}
        onClose={() => {}}
        onNotice={() => {}}
      />,
    );
    expect((await screen.findAllByText(/团队跑/)).length).toBeGreaterThan(0);
    await waitFor(() => {
      expect(apiGetMock).toHaveBeenCalledWith('/api/project/todos/t2/runs');
    });
    expect(await screen.findByText('DAG')).toBeTruthy();
    expect(screen.getByText(/cap-77/)).toBeTruthy();
    // output_ref renders truncated with the full path on the title attr.
    const out = screen.getByTitle('/workflow/prun-x1/build-artifacts');
    expect((out.textContent || '').endsWith('…')).toBe(true);
  });
});

describe('executor helpers (pure)', () => {
  it('executorBody trims, omits blanks and gates on JSON validity', () => {
    expect(executorBody({})).toEqual({ fields: { executor_kind: 'agent' } });
    expect(executorBody({ executor_kind: 'agent', executor_spec: '{}' }).fields)
      .toEqual({ executor_kind: 'agent' });
    expect(executorBody({ executor_kind: 'team', executor_ref: '  x  ', executor_spec: ' {"routes":[]} ' }))
      .toEqual({ fields: { executor_kind: 'team', executor_ref: 'x', executor_spec: '{"routes":[]}' } });
    expect(executorBody({ executor_kind: 'dag', executor_ref: ' ' }).fields).toEqual({ executor_kind: 'dag' });
    expect(executorBody({ executor_kind: 'brain', executor_spec: 'nope' }).error).toBeTruthy();
  });

  it('executorRefText falls back to the agent name / blank', () => {
    expect(executorRefText({ executor_kind: 'agent', agent: 'build' })).toBe('build');
    expect(executorRefText({})).toBe('act');
    expect(executorRefText({ executor_kind: 'team', executor_ref: 'crew' })).toBe('crew');
    expect(executorRefText({ executor_kind: 'dag' })).toBe('');
  });
});
