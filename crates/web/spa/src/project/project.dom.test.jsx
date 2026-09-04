// @vitest-environment jsdom
// Project module DOM smoke (Phase 4): ProjectPanel renders its four tabs from
// a mocked api module (same contract style as todoPanel/team DOM tests),
// goals render markdown bodies, 新建目标 modal POSTs /api/project/goals,
// milestone status Segmented PATCHes, the todos table flattens milestones +
// backlog (未分组), and 详情 opens the runs drawer where a running run can be
// cancelled. Polling is left on real timers — every api mock resolves to the
// same fixture instantly, so silent re-polls are no-ops and tests stay
// deterministic without fake timers.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';

const { apiGetMock, apiPostMock, apiPatchMock, apiDelMock } = vi.hoisted(() => ({
  apiGetMock: vi.fn(),
  apiPostMock: vi.fn(),
  apiPatchMock: vi.fn(),
  apiDelMock: vi.fn(),
}));
vi.mock('../api.js', () => ({
  apiGet: apiGetMock,
  apiPost: apiPostMock,
  apiPatch: apiPatchMock,
  apiDel: apiDelMock,
}));

import '../test/setup-dom.js';
import { ProjectPanel } from './project.jsx';

const T0 = 1700000000000;

const overviewFixture = () => ({
  goals: [
    {
      id: 'g1', title: '发布 1.0', detail_md: '# 标题\n\n- 甲', status: 'active', sort: 0,
      created_at: T0, updated_at: T0,
      milestones: [
        {
          id: 'm1', goal_id: 'g1', title: 'M1 冲刺', detail_md: '', status: 'in_progress', sort: 0,
          created_at: T0, updated_at: T0,
          todos: [
            { id: 't1', milestone_id: 'm1', title: '写发布说明', draft: '草稿内容', plan_md: '# Plan\n步骤', status: 'planned', agent: 'act', active_session_id: null, created_at: T0, updated_at: T0 },
            { id: 't2', milestone_id: 'm1', title: '回归测试', draft: '跑全量', plan_md: null, status: 'draft', agent: 'act', active_session_id: null, created_at: T0, updated_at: T0 },
          ],
        },
      ],
    },
  ],
  backlog: [
    { id: 't9', milestone_id: null, title: '杂项', draft: '未分组任务', plan_md: null, status: 'draft', agent: 'act', active_session_id: null, created_at: T0, updated_at: T0 },
  ],
});

const runsFixture = () => ({
  runs: [
    { id: 'r2', todo_id: 't1', kind: 'execute', version: 2, plan_md: '# Plan v2', output_md: '输出内容', agent: 'act', session_id: 'sess-xyz', status: 'done', started_at: T0, finished_at: T0 + 5000, created_at: T0 },
    { id: 'r1', todo_id: 't1', kind: 'plan', version: 1, plan_md: null, output_md: null, agent: 'act', session_id: null, status: 'running', started_at: T0, finished_at: null, created_at: T0 },
  ],
});

beforeEach(() => {
  apiGetMock.mockReset().mockImplementation((path) => {
    if (path === '/api/project/overview') {
      return Promise.resolve(overviewFixture());
    }
    if (path === '/api/project/todos/t1/runs') {
      return Promise.resolve(runsFixture());
    }
    return Promise.resolve({});
  });
  apiPostMock.mockReset().mockResolvedValue({ ok: true, run_id: 'r9' });
  apiPatchMock.mockReset().mockResolvedValue({ ok: true });
  apiDelMock.mockReset().mockResolvedValue({ deleted: true });
});

// antd 6 Button inserts spaces into two-CJK-char labels (「保 存」) — match
// by textContent with the whitespace normalized. Deliberately NOT getByRole:
// the byRole accessible-name computation is pathologically slow under this
// jsdom+cssinjs setup (50s for a single row-scoped query).
const findButton = (txt, root) => [...(root || document).querySelectorAll('button')]
  .find((b) => (b.textContent || '').replace(/\s+/g, '') === txt);

const mountPanel = () => render(<ProjectPanel onNotice={() => {}} />);

/// antd Tabs panes are wrapped in rc-motion, which REPLACES the pane DOM once
/// right after activation: a node captured across that commit goes stale
/// (detached, React-less) and clicks on it vanish. Settling one macrotask
/// after the switch keeps every subsequent query on the live generation.
const settle = () => act(async () => {
  await new Promise((r) => setTimeout(r, 20));
});
const openTab = async (name) => {
  fireEvent.click(screen.getByRole('tab', { name }));
  await settle();
};

describe('ProjectPanel', () => {
  it('renders the four tabs and the overview counters', async () => {
    mountPanel();
    await screen.findByText('工作流：');
    const labels = [...document.querySelectorAll('.ant-tabs-tab')].map((t) => t.textContent);
    expect(labels).toEqual(['总览', '项目目标', '里程碑', 'TODO']);
    expect(screen.getByText('目标', { exact: true })).toBeTruthy(); // Statistic title
    expect(screen.getByText('未分组 TODO')).toBeTruthy();
  });

  it('goals tab renders markdown detail and archive/delete actions', async () => {
    mountPanel();
    await openTab('项目目标');
    expect(await screen.findByText('发布 1.0')).toBeTruthy();
    // # 标题 became an <h1> inside the .md-body card body.
    const h1 = document.querySelector('.md-body h1');
    expect(h1 && h1.textContent).toBe('标题');
    expect(findButton('归档')).toBeTruthy();
    expect(findButton('删除')).toBeTruthy();
  });

  it('新建目标 modal submits POST /api/project/goals', async () => {
    mountPanel();
    await openTab('项目目标');
    fireEvent.click(await screen.findByText('新建目标'));
    await settle();
    fireEvent.change(screen.getByPlaceholderText('一句话标题'), { target: { value: '新目标' } });
    fireEvent.click(findButton('保存'));
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith(
        '/api/project/goals',
        expect.objectContaining({ title: '新目标', detail_md: '' }),
      );
    });
  });

  it('milestones tab groups by goal and PATCHes status via Segmented', async () => {
    mountPanel();
    await openTab('里程碑');
    expect(await screen.findByText('M1 冲刺')).toBeTruthy();
    fireEvent.click(screen.getByText('已完成'));
    await waitFor(() => {
      expect(apiPatchMock).toHaveBeenCalledWith('/api/project/milestones/m1', { status: 'done' });
    });
  });

  it('todos table flattens milestones + backlog (未分组) with plan checkmark', async () => {
    mountPanel();
    await openTab('TODO');
    expect(await screen.findByText('写发布说明')).toBeTruthy();
    expect(screen.getByText('回归测试')).toBeTruthy();
    expect(screen.getByText('杂项')).toBeTruthy();
    const row = screen.getByText('杂项').closest('tr');
    expect(row && within(row).getByText('未分组')).toBeTruthy();
    const planned = screen.getByText('写发布说明').closest('tr');
    expect(planned && within(planned).getByText('✓')).toBeTruthy();
  });

  it('详情 opens the runs drawer; a running run can be cancelled', async () => {
    mountPanel();
    await openTab('TODO');
    const row = (await screen.findByText('写发布说明')).closest('tr');
    await settle();
    fireEvent.click(findButton('详情', screen.getByText('写发布说明').closest('tr')));
    expect(await screen.findByText('TODO · 写发布说明')).toBeTruthy();
    await waitFor(() => {
      expect(apiGetMock).toHaveBeenCalledWith('/api/project/todos/t1/runs');
    });
    // v2 done run shows its session; v1 running run offers 取消.
    expect(screen.getByText(/sess-xyz/)).toBeTruthy();
    fireEvent.click(findButton('取消'));
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith('/api/project/runs/r1/cancel');
    });
  });
});
