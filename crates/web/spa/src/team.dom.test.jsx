// @vitest-environment jsdom
// Team/topics DOM smoke against the fleet IA panels — fleet/teams.jsx and
// fleet/executions.jsx are the maintained sources behind the 组队/全部执行
// tabs (the pre-fleet top-level copies were deleted); topicDetail.jsx keeps
// its legacy deep-view describe below. Landmarks render from a mocked api
// module — same contract style as queuePanel.dom.test.jsx /
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
import { TopicDetailPanel } from './topicDetail.jsx';
import { ExecutionsPanel as TopicsPanel } from './fleet/executions.jsx';
import { clearCredentials, getState, setState } from './store.js';

const T0 = 1700000000000;

const nodesFixture = {
  nodes: [
    { id: 'n1', name: 'alpha', online: true, kinds: ['agent', 'team'], snapshot: { ready: true, cpu_capacity: 8, active_agent_loops: 2 } },
    { id: 'n2', name: 'beta', online: false, kinds: ['agent', 'team'] },
  ],
};

const agentsFixture = { agents: [{ name: 'act' }, { name: 'explore' }] };

const teamsFixture = {
  teams: [
    {
      name: 't1',
      captain: 'm1',
      members: [
        { id: 'm1', agent: 'act', role: '协调任务并汇总结果', node_id: 'n1', online: true },
        { id: 'm2', agent: 'review', role: '代码评审' },
      ],
    },
  ],
};

const executionsFixture = {
  executions: [
    { id: 'ex-running', kind: 'agent', status: 'running', created_at: T0, node_id: 'n1' },
    { id: 'ex-error', kind: 'team', status: 'error', created_at: T0, node_id: 'n2' },
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
    if (p.startsWith('/api/agents')) {
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
  setState({ page: 'nodes', preselectNode: null, nodes: [], conn: 'init', topicsTeamFilter: null, topicDetail: null });
  installApi();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

/// antd inserts a space inside two-CJK-char buttons ("编 辑"), so match
/// buttons on whitespace-squashed textContent (same trick as
/// agentDetail.dom.test.jsx / fleet.dom.test.jsx).
const findButton = (txt) => screen.getAllByRole('button')
  .find((b) => (b.textContent || '').replace(/\s+/g, '') === txt);

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
  it('renders the team row with captain, member digest and both row actions', async () => {
    render(<TeamPanel onNotice={() => {}} />);
    expect(await screen.findByText('t1')).toBeTruthy();
    expect(screen.getByText('团队组队')).toBeTruthy(); // page header via PAGE_META
    expect(screen.getByText('m1')).toBeTruthy(); // 队长 column
    expect(screen.getByText('act')).toBeTruthy(); // member agent Tag
    expect(screen.getByText(/协调任务并汇总结果 · 在线/)).toBeTruthy(); // digest + member node state
    expect(screen.getByText('review')).toBeTruthy();
    expect(screen.getByText('代码评审')).toBeTruthy();
    expect(findButton('编辑')).toBeTruthy();
    expect(findButton('启动团队')).toBeTruthy();
    expect(findButton('创建团队')).toBeTruthy();
    expect(findButton('刷新')).toBeTruthy();
  });

  it('opens the create-team modal with the member-agent picker fed by /api/agents', async () => {
    render(<TeamPanel onNotice={() => {}} />);
    fireEvent.click(await screen.findByText('创建团队'));
    expect(await screen.findByText('团队成员与职责')).toBeTruthy();
    expect(screen.getByLabelText('团队名称')).toBeTruthy();
    expect(screen.getByLabelText('队长的成员 ID')).toBeTruthy();
    expect(screen.getByPlaceholderText('成员 ID')).toBeTruthy(); // default member row
    fireEvent.mouseDown(screen.getByRole('combobox')); // the member's Agent picker
    // scope to the dropdown options (antd portals them outside the modal and
    // jsdom may render holders twice, so getByText is ambiguous here)
    await waitFor(() => {
      const labels = [...document.querySelectorAll('.ant-select-item-option')]
        .map((o) => o.getAttribute('title') || o.textContent);
      expect(labels).toEqual(expect.arrayContaining(['act', 'explore'])); // from /api/agents
    });
    expect(apiGetMock).toHaveBeenCalledWith('/api/agents');
    fireEvent.click(screen.getByText('添加成员'));
    expect(screen.getAllByPlaceholderText('成员 ID')).toHaveLength(2); // the add action grows the form
    expect(findButton('保存团队')).toBeTruthy();
  });

  it('启动团队 arms the launch modal with the team name and node candidates from /api/nodes', async () => {
    render(<TeamPanel onNotice={() => {}} />);
    fireEvent.click(await screen.findByText('启动团队'));
    expect(await screen.findByText('启动 t1')).toBeTruthy();
    expect(screen.getByText(/整个团队会在同一个执行节点内完成/)).toBeTruthy();
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
    fireEvent.click(screen.getByText('启动团队'));
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
    expect(screen.getByText('舰队全部执行记录与团队过滤')).toBeTruthy(); // page header via PAGE_META
    expect(screen.getAllByText('Agent')).toHaveLength(2); // launch-form kind value + 类型 cell
    expect(screen.getByText('Team')).toBeTruthy();
    expect(screen.getByText('运行中')).toBeTruthy(); // STATUS_META via ui/statusTag
    expect(screen.getByText('失败')).toBeTruthy();
    expect(screen.getByText('在线')).toBeTruthy(); // n1 node state tag
    expect(screen.getByText('离线')).toBeTruthy(); // n2 node state tag
    expect(screen.getByText('启动执行')).toBeTruthy();
  });

  it('filters the list by kind through the 执行类型筛选 select', async () => {
    setState({ page: 'topics' });
    render(<TopicsPanel onNotice={() => {}} />);
    await screen.findByText('ex-running');
    await pickSelectOption(document.querySelector('[aria-label="执行类型筛选"]'), 'Team');
    await waitFor(() => expect(apiGetMock).toHaveBeenCalledWith('/api/executions?limit=50&kind=team'));
    expect(await screen.findByText('ex-error')).toBeTruthy();
    expect(screen.queryByText('ex-running')).toBeNull(); // filtered page replaced the rows
  });

  it('hits cancel then resume on the detail drawer action buttons', async () => {
    setState({ page: 'topics' });
    render(<TopicsPanel onNotice={() => {}} />);
    fireEvent.click(await screen.findByText('ex-running')); // ID link opens ExecutionDetail
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

describe('TopicDetailPanel', () => {
  it('renders the timeline, plan, member results, summary and back button', async () => {
    setState({ page: 'topic_detail', topicDetail: { teamName: 't1', topicId: 'tp1' } });
    render(<TopicDetailPanel onNotice={() => {}} />);
    // The question appears both in the timeline entry and the plan card, and
    // the sub-turn count both in the timeline and the block header.
    expect((await screen.findAllByText('如何拆分模块？')).length).toBeGreaterThan(0);
    expect(screen.getByText('Turn 1')).toBeTruthy();
    expect(await screen.findByText('先摸清边界')).toBeTruthy();
    expect(screen.getByText('n1 · 回答')).toBeTruthy();
    expect(screen.getByText('n2 · 对齐追答')).toBeTruthy();
    expect(screen.getByText('一致同意三分法')).toBeTruthy();
    expect(screen.getAllByText('子轮 1').length).toBeGreaterThan(0);
    expect(screen.getByText('← 返回话题列表')).toBeTruthy();
  });

  it('backs out to the topics list through the store', async () => {
    setState({ page: 'topic_detail', topicDetail: { teamName: 't1', topicId: 'tp1' } });
    render(<TopicDetailPanel onNotice={() => {}} />);
    fireEvent.click(await screen.findByText('← 返回话题列表'));
    expect(getState().page).toBe('topics');
    expect(getState().topicDetail).toBeNull();
  });
});
