// @vitest-environment jsdom
// Team/topics DOM smoke (opencode-team Phase 4): the three new panels render
// their landmarks from a mocked api module — same contract style as
// queuePanel.dom.test.jsx. Everything above the protocol layer runs for
// real, including the store wiring (openTopicsForTeam / openTopicDetail).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';

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

import './test/setup-dom.js';
import { TeamPanel } from './teamPanel.jsx';
import { TopicDetailPanel } from './topicDetail.jsx';
import { TopicsPanel } from './topicsPanel.jsx';
import { clearCredentials, getState, setState } from './store.js';

const T0 = 1700000000000;

const nodesFixture = {
  nodes: [
    { id: 'n1', name: 'alpha', status: 'online' },
    { id: 'n2', name: 'beta', status: 'idle' },
  ],
};

const teamsFixture = {
  teams: [
    {
      name: 't1',
      captain: { node_id: 'n1', name: 'alpha' },
      members: [
        { node_id: 'n1', name: 'alpha', capabilities: ['rust', 'web'], profiled_at: T0 },
        { node_id: 'n2', name: 'beta', capabilities: [], profiled_at: null },
      ],
      created_at: T0,
      updated_at: T0,
    },
  ],
};

const topicsFixture = {
  topics: [
    { topic_id: 'tp1', team_name: 't1', title: '调研话题', requirement: 'r', status: 'executing', finish_reason: null, created_at: T0, finished_at: null },
    { topic_id: 'tp2', team_name: 't1', title: '完结话题', requirement: 'r', status: 'finished', finish_reason: 'max_turns', created_at: T0, finished_at: T0 },
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
    if (p.startsWith('/api/teams/t1/topics/tp1')) {
      return Promise.resolve(detailFixture);
    }
    if (p.startsWith('/api/teams/t1/topics')) {
      return Promise.resolve({ topics: topicsFixture.topics });
    }
    if (p.startsWith('/api/topics')) {
      return Promise.resolve(topicsFixture);
    }
    if (p.startsWith('/api/teams')) {
      return Promise.resolve(teamsFixture);
    }
    return Promise.resolve({});
  });
  apiPostMock.mockReset().mockResolvedValue({ ok: true, accepted: true });
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

describe('TeamPanel', () => {
  it('renders the team row with captain, capability digest and all row actions', async () => {
    render(<TeamPanel onNotice={() => {}} />);
    expect(await screen.findByText('t1')).toBeTruthy();
    expect(screen.getByText('alpha')).toBeTruthy();
    expect(screen.getByText('rust / web')).toBeTruthy();
    expect(screen.getByText('改队长')).toBeTruthy();
    expect(screen.getByText('成员管理')).toBeTruthy();
    expect(screen.getByText('发起话题')).toBeTruthy();
    expect(screen.getByText('查看话题')).toBeTruthy();
    expect(screen.getByText('能力画像')).toBeTruthy();
  });

  it('opens the create-team modal with captain candidates from /api/nodes', async () => {
    render(<TeamPanel onNotice={() => {}} />);
    fireEvent.click(await screen.findByText('新建团队'));
    expect(await screen.findByText('队长（单选）')).toBeTruthy();
    expect(screen.getAllByText('alpha').length).toBeGreaterThan(0);
    expect(screen.getByText('beta')).toBeTruthy();
  });

  it('查看话题 arms the topics tab with the team filter', async () => {
    render(<TeamPanel onNotice={() => {}} />);
    fireEvent.click(await screen.findByText('查看话题'));
    expect(getState().page).toBe('topics');
    expect(getState().topicsTeamFilter).toBe('t1');
  });

  it('dispatches a profiling task on confirm', async () => {
    render(<TeamPanel onNotice={() => {}} />);
    await screen.findByText('t1');
    fireEvent.click(screen.getByText('能力画像'));
    // antd inserts a space inside two-CJK-char buttons ("派 发"); anchored so
    // the Popconfirm title 派发能力画像？ does not match.
    fireEvent.click(await screen.findByText(/^派\s*发$/));
    await act(async () => {});
    expect(apiPostMock).toHaveBeenCalledWith('/api/teams/t1/profile', {});
  });
});

describe('TopicsPanel', () => {
  it('renders both topics with status tags and row actions', async () => {
    setState({ page: 'topics' });
    render(<TopicsPanel onNotice={() => {}} />);
    expect(await screen.findByText('调研话题')).toBeTruthy();
    expect(screen.getByText('完结话题')).toBeTruthy();
    expect(screen.getByText('执行中')).toBeTruthy();
    expect(screen.getByText('轮数上限')).toBeTruthy();
    expect(screen.getAllByText('详情')).toHaveLength(2);
    expect(screen.getByText('取消')).toBeTruthy(); // executing only
    expect(screen.getByText('恢复')).toBeTruthy(); // finished non-complete only
    expect(screen.getByText('max_turns')).toBeTruthy();
  });

  it('filters by team when armed through openTopicsForTeam', async () => {
    const { openTopicsForTeam } = await import('./store.js');
    openTopicsForTeam('t1');
    expect(getState().topicsTeamFilter).toBe('t1');
    render(<TopicsPanel onNotice={() => {}} />);
    await screen.findByText('调研话题');
    expect(apiGetMock).toHaveBeenCalledWith('/api/topics?team=t1');
  });

  it('hits cancel then resume on the action buttons', async () => {
    setState({ page: 'topics' });
    render(<TopicsPanel onNotice={() => {}} />);
    await screen.findByText('调研话题');
    fireEvent.click(screen.getByText('取消'));
    fireEvent.click(await screen.findByText('取消话题'));
    await act(async () => {});
    expect(apiPostMock).toHaveBeenCalledWith('/api/teams/t1/topics/tp1/cancel', {});
    fireEvent.click(screen.getByText('恢复'));
    await act(async () => {});
    expect(apiPostMock).toHaveBeenCalledWith('/api/teams/t1/topics/tp2/resume', {});
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
    expect(screen.getByText('先摸清边界')).toBeTruthy();
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
