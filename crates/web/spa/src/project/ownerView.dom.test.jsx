// @vitest-environment jsdom
// Owner 视角 page DOM smoke (iteration 4): the goal rollup renders per-goal
// health badges + milestone done/total mini bars, the 待人工介入 section
// lists failed TODOs whose 处理 button opens the panel-owned TodoDrawer,
// stale planned todos surface the 阻塞提示, and the section disappears
// entirely when there is nothing to intervene on. goalHealth /
// BLOCKED_AFTER_MS / isStalePlanned are asserted as pure functions here too.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';

const { apiGetMock } = vi.hoisted(() => ({ apiGetMock: vi.fn() }));
vi.mock('../api.js', () => ({ apiGet: apiGetMock }));

import '../test/setup-dom.js';
import {
  BLOCKED_AFTER_MS,
  OwnerViewPanel,
  attentionRows,
  goalHealth,
  isStalePlanned,
} from './ownerViewPanel.jsx';

const HOUR = 60 * 60 * 1000;

const overviewFixture = () => {
  const now = Date.now();
  const stale = now - 25 * HOUR; // planned but untouched for 25h → blocked
  const fresh = now - 1 * HOUR;
  return {
    goals: [
      {
        id: 'g1', title: '发布 1.0', status: 'active',
        milestones: [
          {
            id: 'm1', goal_id: 'g1', title: 'M1 冲刺', status: 'in_progress',
            todos: [
              { id: 't1', milestone_id: 'm1', title: '写发布说明', status: 'done', created_at: fresh, updated_at: fresh },
              { id: 't2', milestone_id: 'm1', title: '回归测试', status: 'failed', created_at: fresh, updated_at: fresh },
            ],
          },
          {
            id: 'm2', goal_id: 'g1', title: 'M2 打磨', status: 'planned',
            todos: [
              { id: 't3', milestone_id: 'm2', title: '性能压测', status: 'planned', created_at: stale, updated_at: stale },
              { id: 't4', milestone_id: 'm2', title: '文档补全', status: 'planned', created_at: fresh, updated_at: fresh },
            ],
          },
        ],
      },
      {
        id: 'g2', title: '站点改版', status: 'active',
        milestones: [
          {
            id: 'm3', goal_id: 'g2', title: 'M1 设计稿', status: 'done',
            todos: [
              { id: 't5', milestone_id: 'm3', title: '首页视觉', status: 'done', created_at: fresh, updated_at: fresh },
            ],
          },
        ],
      },
    ],
    backlog: [
      { id: 't9', milestone_id: null, title: '杂项', status: 'draft', created_at: fresh, updated_at: fresh },
    ],
  };
};

beforeEach(() => {
  apiGetMock.mockReset().mockImplementation((path) => {
    if (path === '/api/project/overview') {
      return Promise.resolve(overviewFixture());
    }
    if (path === '/api/project/todos/t2/runs') {
      return Promise.resolve({ runs: [] });
    }
    return Promise.resolve({});
  });
});

// antd 6 Button inserts a space into two-CJK-char labels (「处 理」) — match
// by textContent with whitespace normalized (same helper as project.dom.test).
const findButton = (txt, root) => [...(root || document).querySelectorAll('button')]
  .find((b) => (b.textContent || '').replace(/\s+/g, '') === txt);

const mountPanel = () => render(<OwnerViewPanel onNotice={() => {}} />);

describe('OwnerViewPanel', () => {
  it('rolls goals up with health badges and milestone mini bars', async () => {
    mountPanel();
    expect(await screen.findByText('发布 1.0')).toBeTruthy();
    // g1 has a failed todo → 需介入; g2 is all done → 已完成.
    expect(screen.getByText('需介入')).toBeTruthy();
    expect(screen.getByText('已完成')).toBeTruthy();
    // Mini bars: m1 1/2, m2 0/2, m3 1/1.
    expect(screen.getByText('1/2')).toBeTruthy();
    expect(screen.getByText('0/2')).toBeTruthy();
    expect(screen.getByText('1/1')).toBeTruthy();
    // Backlog count strip.
    expect(screen.getByText('未分组 TODO（backlog）：', { exact: false })).toBeTruthy();
  });

  it('待人工介入 lists the failed todo; 处理 opens the TodoDrawer', async () => {
    mountPanel();
    expect(await screen.findByText('待人工介入')).toBeTruthy();
    const failedRow = screen.getByText('回归测试').closest('tr');
    expect(failedRow && within(failedRow).getByText('发布 1.0 / M1 冲刺')).toBeTruthy();
    // Failed rows carry a relative 失败于 timestamp.
    expect(failedRow.textContent.includes('失败于')).toBe(true);
    const btn = findButton('处理', failedRow);
    expect(btn).toBeTruthy();
    fireEvent.click(btn);
    expect(await screen.findByText('TODO · 回归测试')).toBeTruthy();
    await waitFor(() => {
      expect(apiGetMock).toHaveBeenCalledWith('/api/project/todos/t2/runs');
    });
  });

  it('flags planned todos quiet past 24h as 已规划长时间未执行', async () => {
    mountPanel();
    const staleRow = (await screen.findByText('性能压测')).closest('tr');
    expect(staleRow.textContent.includes('已规划长时间未执行')).toBe(true);
    // The freshly-planned todo is NOT flagged (absent from the page: goal
    // cards list milestone titles, not todo titles).
    expect(screen.queryByText('文档补全')).toBeNull();
  });

  it('hides the 待人工介入 section when there is nothing to intervene on', async () => {
    apiGetMock.mockImplementation((path) => Promise.resolve(
      path === '/api/project/overview' ? overviewFixtureClean() : { runs: [] },
    ));
    mountPanel();
    expect(await screen.findByText('发布 1.0')).toBeTruthy();
    expect(screen.queryByText('待人工介入')).toBeNull();
  });

  it('renders the 暂无项目目标 Empty for a project without goals', async () => {
    apiGetMock.mockImplementation((path) => Promise.resolve(
      path === '/api/project/overview' ? { goals: [], backlog: [] } : {},
    ));
    mountPanel();
    expect(await screen.findByText('暂无项目目标')).toBeTruthy();
  });
});

// All-done, nothing failed/blocked variant for the hidden-section case.
function overviewFixtureClean() {
  const now = Date.now();
  return {
    goals: [
      {
        id: 'g1', title: '发布 1.0', status: 'active',
        milestones: [
          {
            id: 'm1', goal_id: 'g1', title: 'M1 冲刺', status: 'done',
            todos: [{ id: 't1', milestone_id: 'm1', title: '写发布说明', status: 'done', updated_at: now }],
          },
        ],
      },
    ],
    backlog: [],
  };
}

describe('owner-view derivations (pure)', () => {
  const goalOf = (statuses) => ({ id: 'g', title: 'g', milestones: [{ id: 'm', title: 'm', todos: statuses.map((status) => ({ id: status, status })) }] });

  it('goalHealth maps the four health bands in priority order', () => {
    expect(goalHealth(goalOf(['done', 'done']))).toEqual({ status: 'success', label: '已完成' });
    expect(goalHealth(goalOf(['done', 'failed']))).toEqual({ status: 'error', label: '需介入' });
    expect(goalHealth(goalOf(['failed', 'running']))).toEqual({ status: 'error', label: '需介入' });
    expect(goalHealth(goalOf(['planned', 'running']))).toEqual({ status: 'processing', label: '进行中' });
    expect(goalHealth(goalOf(['draft', 'planned']))).toEqual({ status: 'default', label: '规划中' });
    // No todos at all → nothing achieved yet, stays 规划中.
    expect(goalHealth(goalOf([]))).toEqual({ status: 'default', label: '规划中' });
    expect(goalHealth(null)).toEqual({ status: 'default', label: '规划中' });
  });

  it('exposes the 24h threshold and isStalePlanned boundaries', () => {
    expect(BLOCKED_AFTER_MS).toBe(24 * 60 * 60 * 1000);
    const now = Date.now();
    expect(isStalePlanned({ status: 'planned', updated_at: now - 25 * HOUR }, now)).toBe(true);
    expect(isStalePlanned({ status: 'planned', updated_at: now - 23 * HOUR }, now)).toBe(false);
    expect(isStalePlanned({ status: 'planned' }, now)).toBe(false);
    expect(isStalePlanned({ status: 'draft', updated_at: now - 48 * HOUR }, now)).toBe(false);
  });

  it('attentionRows lists failed first, then blocked, tagged per row', () => {
    const rows = attentionRows(overviewFixture(), Date.now());
    expect(rows.map((r) => [r.attention, r.id])).toEqual([['failed', 't2'], ['blocked', 't3']]);
    expect(attentionRows({ goals: [], backlog: [] })).toEqual([]);
  });
});
