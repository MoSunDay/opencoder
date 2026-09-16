// @vitest-environment jsdom
// Team/topics DOM smoke against the fleet IA panels — fleet/teams.jsx and
// fleet/executions.jsx are the maintained sources behind the 组队/全部执行
// tabs (the pre-fleet top-level copies were deleted). Landmarks render from
// a mocked api module — same contract style as queuePanel.dom.test.jsx /
// fleet/fleet.dom.test.jsx. Everything above the protocol layer (api.js
// requests + the sse.js event stream) runs for real, including the
// ExecutionDetail drawer where 取消/恢复 live now.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

const { apiGetMock, apiPostMock, apiPatchMock } = vi.hoisted(() => ({
  apiGetMock: vi.fn(),
  apiPostMock: vi.fn(),
  apiPatchMock: vi.fn(),
}));
vi.mock('./api.js', () => ({
  apiGet: apiGetMock,
  apiPost: apiPostMock,
  apiPatch: apiPatchMock,
  apiDel: vi.fn(),
}));
// ExecutionDetail consumes a live event stream; sse.js sits on the same
// protocol/transport layer as api.js, so stub it with a no-op stream instead
// of letting fetch-retry timers race the assertions.
vi.mock('./sse.js', () => ({ openStream: () => ({ abort() {} }) }));

import './test/setup-dom.js';
import { FleetTeamsPanel as TeamPanel } from './fleet/teams.jsx';
import { ExecutionsPanel as TopicsPanel } from './fleet/executions.jsx';
import { clearCredentials, getState, setState } from './store.js';

const T0 = 1700000000000;

const nodesFixture = {
  nodes: [
    { id: 'n1', name: 'alpha', online: true, kinds: ['agent', 'team'], snapshot: { ready: true, cpu_capacity: 8, active_agent_loops: 2 } },
    { id: 'n2', name: 'beta', online: false, kinds: ['agent', 'team'] },
  ],
};

const agentsFixture = { agents: [
  { agent: 'act', capabilities: [{ id: 'c1', summary: '执行任务' }] },
  { agent: 'explore', capabilities: [] },
] };

const teamsFixture = {
  teams: [
    { name: 't1', captain: 'act', members: [{ agent: 'act' }, { agent: 'review' }] },
    { name: 't2', captain: 'explore', members: [{ agent: 'explore' }] },
  ],
};

const executionsFixture = {
  executions: [
    { id: 'ex-running', kind: 'agent', name: 'coder-x', status: 'running', created_at: T0, node_id: 'n1' },
    { id: 'ex-maint', kind: 'maintenance', status: 'done', created_at: T0, node_id: 'n1' },
    { id: 'ex-error', kind: 'team', name: 't1', status: 'error', created_at: T0, node_id: 'n2' },
  ],
};

const detailFixture = {
  topic: {
    topic_id: 'tp1', team_name: 't1', title: '调研话题', status: 'executing',
    finish_reason: null, created_at: T0, finished_at: null,
    captain: { node_id: 'n1', name: 'alpha' },
    members: [{ node_id: 'n1', name: 'alpha' }, { node_id: 'n2', name: 'beta' }],
    turns: [], final_summary: null,
  },
  turns: [
    {
      turn: 1,
      plan: { turn: 1, question: '如何拆分模块？', participants: ['n1', 'n2'], rationale: '先摸清边界' },
      sub_turns: [
        {
          sub_turn: 1,
          results: [
            { node_id: 'n1', turn: 1, sub_turn: 1, kind: 'answer', answer: '分三个 crate', ok: true, error: null, created_at: T0 },
            { node_id: 'n2', turn: 1, sub_turn: 1, kind: 'alignment', answer: '同意该拆分', ok: true, error: null, created_at: T0 },
          ],
          summary: { summary: '一致同意三分法', aligned: true, ambiguities: [], created_at: T0 },
        },
      ],
    },
  ],
};

const installApi = () => {
  apiGetMock.mockReset().mockImplementation((path) => {
    const p = String(path);
    if (p.startsWith('/api/nodes')) {
      return Promise.resolve(nodesFixture);
    }
    if (p.startsWith('/api/brain/agents')) {
      return Promise.resolve(agentsFixture);
    }
    if (p.startsWith('/api/executions?')) {
      // the kind filter narrows the list page, mirroring the real endpoint
      const rows = p.includes('kind=team')
        ? executionsFixture.executions.filter((row) => row.kind === 'team')
        : executionsFixture.executions;
      return Promise.resolve({ executions: rows });
    }
    if (p.startsWith('/api/executions/')) {
      return Promise.resolve({}); // drawer detail GET / message pages: empty payloads are valid
    }
    if (p.startsWith('/api/teams/t1/topics/tp1')) {
      return Promise.resolve(detailFixture);
    }
    if (p.startsWith('/api/teams')) {
      return Promise.resolve(teamsFixture);
    }
    return Promise.resolve({});
  });
  // POSTs echo their body so a dispatched execution carries its id into the detail drawer.
  apiPostMock.mockReset().mockImplementation(async (_path, body) => ({ ...(body || {}), ok: true }));
  apiPatchMock.mockReset().mockResolvedValue({ team: teamsFixture.teams[0] });
};

beforeEach(() => {
  localStorage.clear();
  clearCredentials();
  setState({ page: 'nodes', preselectNode: null, nodes: [], conn: 'init' });
  installApi();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

/// antd inserts a space inside two-CJK-char buttons ("编 辑"), so match
/// buttons on whitespace-squashed textContent (same trick as
/// agentDetail.dom.test.jsx / fleet.dom.test.jsx). 混排文案（如「启动 Team」）
/// 自带空格，指针也一并去空白再比。
const findButton = (txt) => screen.getAllByRole('button')
  .find((b) => (b.textContent || '').replace(/\s+/g, '') === txt.replace(/\s+/g, ''));

/// Open an antd Select and pick the dropdown option with the exact label —
/// the same interaction helper agentDetail.dom.test.jsx uses (options render
/// in a body-level portal, so scope the search to .ant-select-item-option).
const pickSelectOption = async (selectEl, label) => {
  await act(async () => {
    fireEvent.mouseDown(selectEl);
  });
  const option = await waitFor(() => {
    const hit = [...document.querySelectorAll('.ant-select-item-option')]
      .find((o) => o.getAttribute('title') === label || o.textContent === label);
    expect(hit).toBeTruthy();
    return hit;
  });
  await act(async () => {
    fireEvent.click(option);
  });
  return option;
};

describe('TeamPanel', () => {
  it('renders the team row with captain, member agent tags and both row actions', async () => {
    render(<TeamPanel onNotice={() => {}} />);
    expect(await screen.findByText('t1')).toBeTruthy();
    expect(screen.queryByText('团队组队')).toBeNull(); // 团队页不再显示冗余页头
    expect(screen.getAllByText('act')).toHaveLength(2); // 队长 cell（agent 名）+ member Tag
    expect(screen.getByText('review')).toBeTruthy(); // member agent Tag
    expect(screen.queryByText(/协调任务并汇总结果/)).toBeNull(); // 职责由服务端固化，不再随成员下发
    expect(findButton('编辑')).toBeTruthy();
    expect(findButton('启动 Team')).toBeTruthy();
    expect(findButton('创建 Team')).toBeTruthy();
    expect(findButton('刷新')).toBeTruthy();
  });

  it('filters teams by name through the controlled team-search box (case-insensitive)', async () => {
    render(<TeamPanel onNotice={() => {}} />);
    expect(await screen.findByText('t1')).toBeTruthy();
    // 大写输入命中小写 team 名：过滤忽略大小写，t2 行在、t1 行消失。
    fireEvent.change(screen.getByLabelText('team-search'), { target: { value: 'T2' } });
    await waitFor(() => expect(screen.queryByText('t1')).toBeNull());
    expect(screen.getByText('t2')).toBeTruthy();
    // 清空搜索后列表恢复。
    fireEvent.change(screen.getByLabelText('team-search'), { target: { value: '' } });
    expect(await screen.findByText('t1')).toBeTruthy();
    expect(screen.getByText('t2')).toBeTruthy();
  });

  it('opens the create-team modal with the agent pickers fed by /api/brain/agents', async () => {
    render(<TeamPanel onNotice={() => {}} />);
    fireEvent.click(await screen.findByText('创建 Team'));
    expect(await screen.findByText('Team 成员')).toBeTruthy();
    expect(screen.getByLabelText('Team 名称')).toBeTruthy();
    expect(screen.getByLabelText('队长')).toBeTruthy();
    expect(screen.getByLabelText('队员')).toBeTruthy(); // multiple Select
    expect(screen.queryByPlaceholderText('成员 ID')).toBeNull(); // 成员身份即 agent，不再手填 ID
    await act(async () => {
      fireEvent.mouseDown(screen.getByLabelText('队长').closest('.ant-select'));
    });
    // scope to the dropdown options (antd portals them outside the modal and
    // jsdom may render holders twice, so getByText is ambiguous here)
    await waitFor(() => {
      const labels = [...document.querySelectorAll('.ant-select-item-option')]
        .map((o) => o.getAttribute('title') || o.textContent);
      expect(labels).toEqual(expect.arrayContaining(['act', 'explore'])); // from /api/brain/agents
    });
    expect(apiGetMock).toHaveBeenCalledWith('/api/brain/agents');
    expect(findButton('保存 Team')).toBeTruthy();
  });

  it('启动 Team arms the launch modal with the team name and node candidates from /api/nodes', async () => {
    render(<TeamPanel onNotice={() => {}} />);
    fireEvent.click((await screen.findAllByText('启动 Team'))[0]); // 两行各有同名操作按钮
    expect(await screen.findByText('启动 t1')).toBeTruthy();
    expect(screen.getByText(/整个 Team 会在同一个执行节点内完成/)).toBeTruthy();
    expect(screen.getByLabelText('任务要求')).toBeTruthy();
    await act(async () => {
      fireEvent.mouseDown(screen.getByRole('combobox')); // 执行节点 picker
    });
    // the '' option is both the selected value and a dropdown option
    expect((await screen.findAllByText('自动调度（活跃 loop / CPU 最低）')).length).toBeGreaterThan(1);
    expect(screen.getByText('alpha · 2 loops / 8 CPU')).toBeTruthy(); // node label from the snapshot
    expect(apiGetMock).toHaveBeenCalledWith('/api/nodes');
  });

  it('dispatches a team execution on confirm and opens its detail drawer', async () => {
    render(<TeamPanel onNotice={() => {}} />);
    await screen.findByText('t1');
    fireEvent.click(findButton('启动 Team')); // 行操作按钮按去空白匹配唯一载体
    expect(await screen.findByText('启动 t1')).toBeTruthy();
    fireEvent.change(screen.getByLabelText('任务要求'), { target: { value: '准备发布' } });
    // antd inserts a space inside two-CJK-char buttons ("启 动"), so match the
    // squashed text the same way fleet.dom.test.jsx does.
    const submit = [...document.querySelectorAll('.ant-modal button')]
      .find((button) => button.textContent.replace(/\s+/g, '') === '启动');
    expect(submit).toBeTruthy();
    fireEvent.click(submit);
    await act(async () => {});
    expect(apiPostMock).toHaveBeenCalledWith('/api/executions', expect.objectContaining({
      kind: 'team',
      target: 't1',
      input: { prompt: '准备发布' },
      id: expect.stringMatching(/^team-/),
    }));
    expect(await screen.findByText(/^team-[a-f0-9]{32}$/)).toBeTruthy(); // drawer title = dispatched id
    expect(screen.getByText('刷新明细')).toBeTruthy();
  });
});

describe('TopicsPanel', () => {
  it('renders both executions with type labels, node state tags and status tags', async () => {
    setState({ page: 'topics' });
    render(<TopicsPanel onNotice={() => {}} />);
    expect(await screen.findByText('ex-running')).toBeTruthy();
    expect(screen.getByText('ex-error')).toBeTruthy();
    expect(screen.queryByText('舰队全部执行记录与团队过滤')).toBeNull(); // 全部执行页不再显示冗余页头
    expect(screen.getAllByText('Agent')).toHaveLength(1); // 类型 cell（启动表单已收进 Modal，默认不渲染）
    expect(screen.getByText('维护执行')).toBeTruthy(); // maintenance 类型列显示中文标签
    expect(screen.getByText('Team')).toBeTruthy();
    expect(screen.getByText('运行中')).toBeTruthy(); // STATUS_META via ui/statusTag
    expect(screen.getByText('失败')).toBeTruthy();
    expect(screen.getAllByText('在线')).toHaveLength(2); // n1 node state tag（agent + maintenance 两行同节点）
    expect(screen.getByText('离线')).toBeTruthy(); // n2 node state tag
    expect(screen.queryByRole('button', { name: '启动执行' })).toBeNull();
    expect(screen.queryByRole('button', { name: '加载更早的执行' })).toBeNull();
  });

  it('filters the list by kind through the 执行类型筛选 select', async () => {
    setState({ page: 'topics' });
    render(<TopicsPanel onNotice={() => {}} />);
    await screen.findByText('ex-running');
    await pickSelectOption(document.querySelector('[aria-label="执行类型筛选"]'), 'Team');
    await waitFor(() => expect(apiGetMock).toHaveBeenCalledWith('/api/executions?limit=50&kind=team'));
    expect(await screen.findByText('ex-error')).toBeTruthy();
    expect(screen.queryByText('ex-running')).toBeNull(); // filtered page replaced the rows
    expect(screen.queryByText('ex-maint')).toBeNull(); // maintenance 行同样被 Team 筛选滤掉
  });

  it('renders the name column with a dash fallback and titles the drawer by name', async () => {
    setState({ page: 'topics' });
    render(<TopicsPanel onNotice={() => {}} />);
    expect(screen.getByRole('columnheader', { name: '名称' })).toBeTruthy();
    await screen.findByText('coder-x'); // dispatch-time name rendered as-is
    const unnamed = screen.getByText('ex-maint').closest('tr');
    expect(unnamed.textContent).toContain('-'); // missing name falls back to '-'
  });

  it('hits cancel then resume on the detail drawer action buttons', async () => {

    setState({ page: 'topics' });
    render(<TopicsPanel onNotice={() => {}} />);
    fireEvent.click(await screen.findByText('ex-running')); // ID link opens ExecutionDetail
    expect(document.querySelector('.ant-drawer-title')?.textContent).toBe('coder-x (ex-running)');
    expect(findButton('刷新明细')).toBeTruthy();
    expect(findButton('取消（终止）').disabled).toBe(false); // running → cancel armed
    expect(findButton('在原节点恢复').disabled).toBe(true); // running → resume disarmed
    fireEvent.click(findButton('取消（终止）'));
    await act(async () => {});
    expect(apiPostMock).toHaveBeenCalledWith('/api/executions/ex-running/commands', { action: 'cancel', input: {} });
    fireEvent.click(findButton('ex-error')); // switch the drawer to the failed run
    await waitFor(() => expect(findButton('在原节点恢复').disabled).toBe(false));
    expect(findButton('取消（终止）').disabled).toBe(true); // error → cancel disarmed
    fireEvent.click(findButton('在原节点恢复'));
    await act(async () => {});
    expect(apiPostMock).toHaveBeenCalledWith('/api/executions/ex-error/commands', { action: 'resume', input: {} });
  });
});
