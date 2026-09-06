// @vitest-environment jsdom
// 进展 page DOM smoke (iteration 4): the milestone progress rollup renders
// done/total + percent per milestone card with goal context, the live list
// keeps running/queued rows and drops done ones (todosTab busy 口径), an
// empty project renders the 暂无项目目标 Empty, and the recent-executions
// table opens ExecutionDetail on row click. apiGet is routed by URL
// (overview + kind=project executions page); ExecutionDetail is a stub.
// Polling stays on real timers — every mock resolves instantly, so silent
// re-polls are deterministic no-ops (same contract as project.dom.test).

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/react';
import dayjs from 'dayjs';
import 'dayjs/locale/zh-cn';

const { apiGetMock } = vi.hoisted(() => ({ apiGetMock: vi.fn() }));
vi.mock('../api.js', () => ({ apiGet: apiGetMock }));
vi.mock('../fleet/detail.jsx', () => ({ ExecutionDetail: ({ id }) => <div>execution-detail:{id}</div> }));

import '../test/setup-dom.js';
import { ProgressPanel, isTodoLive, liveTodos, milestoneProgress } from './progressPanel.jsx';

// The app sets the dayjs locale at main.jsx import time; a standalone panel
// test has to mirror it so TimeText fromNow renders 「… 前」.
dayjs.locale('zh-cn');

const T0 = 1700000000000;

const overviewFixture = () => ({
  goals: [
    {
      id: 'g1', title: '发布 1.0', status: 'active',
      milestones: [
        {
          id: 'm1', goal_id: 'g1', title: 'M1 冲刺', status: 'in_progress',
          todos: [
            { id: 't1', milestone_id: 'm1', title: '写发布说明', status: 'done', created_at: T0, updated_at: T0 },
            { id: 't2', milestone_id: 'm1', title: '回归测试', status: 'running', execution: { id: 'project-t2', kind: 'project', node_id: 'node-a', status: 'running', started_at: T0, created_at: T0 }, created_at: T0, updated_at: T0 },
          ],
        },
        {
          id: 'm2', goal_id: 'g1', title: 'M2 打磨', status: 'planned',
          todos: [
            { id: 't3', milestone_id: 'm2', title: '性能压测', status: 'draft', created_at: T0, updated_at: T0 },
            { id: 't4', milestone_id: 'm2', title: '文档补全', status: 'draft', created_at: T0, updated_at: T0 },
            { id: 't5', milestone_id: 'm2', title: '清理脚本', status: 'draft', created_at: T0, updated_at: T0 },
          ],
        },
      ],
    },
  ],
  backlog: [
    { id: 't9', milestone_id: null, title: '排队杂项', status: 'planned', execution: { id: 'project-t9', kind: 'project', status: 'pending', created_at: T0 }, created_at: T0, updated_at: T0 },
  ],
});

// The kind=project executions page; per-test overridable (empty by default).
let execPage = { executions: [], next_cursor: null };

beforeEach(() => {
  execPage = { executions: [], next_cursor: null };
  apiGetMock.mockReset().mockImplementation((path) => {
    if (path === '/api/project/overview') {
      return Promise.resolve(overviewFixture());
    }
    if (typeof path === 'string' && path.startsWith('/api/executions') && path.includes('kind=project')) {
      return Promise.resolve(execPage);
    }
    return Promise.resolve({});
  });
});

const mountPanel = () => render(<ProgressPanel onNotice={() => {}} />);

describe('ProgressPanel', () => {
  it('renders the milestone progress rollup (done/total + percent + goal context)', async () => {
    mountPanel();
    expect(await screen.findByText('M1 冲刺')).toBeTruthy();
    // PageShell header desc from PAGE_META.progress.
    expect(screen.getByText('里程碑进度、进行中 TODO 与最近项目执行')).toBeTruthy();
    // m1: 1/2 done → 50%; m2: 0/3 → 0%.
    expect(screen.getByText('1/2')).toBeTruthy();
    expect(screen.getByText('50%')).toBeTruthy();
    expect(screen.getByText('0/3')).toBeTruthy();
    expect(screen.getByText('0%')).toBeTruthy();
    // Cards list the owning goal title.
    expect(screen.getAllByText('发布 1.0').length).toBeGreaterThan(0);
    expect(screen.getByText('M2 打磨')).toBeTruthy();
  });

  it('live list keeps running/queued rows with context and drops done rows', async () => {
    mountPanel();
    expect(await screen.findByText('进行中 TODO')).toBeTruthy();
    const running = screen.getByText('回归测试').closest('tr');
    expect(running).toBeTruthy();
    expect(within(running).getByText('发布 1.0 / M1 冲刺')).toBeTruthy();
    // Relative elapsed text from execution.started_at (dayjs zh-cn fromNow).
    expect(/前$/.test(running.textContent || '')).toBe(true);
    // Backlog row rides the same busy 口径 (execution pending) and shows 未分组.
    const queued = screen.getByText('排队杂项').closest('tr');
    expect(queued && within(queued).getByText('未分组')).toBeTruthy();
    // The done todo is NOT part of the live list.
    expect(screen.queryByText('写发布说明')).toBeNull();
  });

  it('renders the 暂无项目目标 Empty for a project without goals', async () => {
    apiGetMock.mockImplementation((path) => Promise.resolve(
      path === '/api/project/overview' ? { goals: [], backlog: [] } : { executions: [] },
    ));
    mountPanel();
    expect(await screen.findByText('暂无项目目标')).toBeTruthy();
    expect(screen.queryByText('M1 冲刺')).toBeNull();
  });

  it('recent project executions render and a row click opens ExecutionDetail', async () => {
    execPage = {
      executions: [
        { id: 'project-t2', kind: 'project', node_id: 'node-a', status: 'running', created_at: T0 },
      ],
      next_cursor: null,
    };
    mountPanel();
    expect(await screen.findByText('最近项目执行')).toBeTruthy();
    const id = await screen.findByText('project-t2');
    const row = id.closest('tr');
    expect(row && within(row).getByText('node-a')).toBeTruthy();
    fireEvent.click(id);
    expect(await screen.findByText('execution-detail:project-t2')).toBeTruthy();
  });
});

describe('progress derivations (pure)', () => {
  it('milestoneProgress rolls up done/total/percent and zero-total stays 0', () => {
    expect(milestoneProgress({ todos: [{ status: 'done' }, { status: 'done' }, { status: 'running' }] }))
      .toEqual({ done: 2, total: 3, percent: 67 });
    expect(milestoneProgress({ todos: [] })).toEqual({ done: 0, total: 0, percent: 0 });
    expect(milestoneProgress(null)).toEqual({ done: 0, total: 0, percent: 0 });
  });

  it('isTodoLive/liveTodos follow the todosTab busy 口径', () => {
    expect(isTodoLive({ status: 'running' })).toBe(true);
    expect(isTodoLive({ status: 'planned', execution: { status: 'pending' } })).toBe(true);
    expect(isTodoLive({ status: 'planned', execution: { status: 'cancelling' } })).toBe(true);
    expect(isTodoLive({ status: 'done' })).toBe(false);
    expect(isTodoLive({ status: 'draft', execution: { status: 'done' } })).toBe(false);
    expect(liveTodos(overviewFixture()).map((t) => t.id)).toEqual(['t2', 't9']);
  });
});
