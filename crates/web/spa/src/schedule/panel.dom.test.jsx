// @vitest-environment jsdom
// schedule/panel.dom.test.jsx —「调度」页 DOM 契约：定义列表（GET
// /api/schedules）、空态、错误通知、只读提示（schedules.json 事实源）与
// 触发历史 Drawer（GET /api/schedules/:id/runs）的三种台账 status。
// 只断言可观测 DOM；api.js 模块级 mock，sse.js 不涉及（本页无事件流）。

import '../test/setup-dom.js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { err } from '../notice.js';
import { SchedulePanel } from './panel.jsx';

const { apiGetMock } = vi.hoisted(() => ({ apiGetMock: vi.fn() }));
vi.mock('../api.js', () => ({
  apiGet: apiGetMock,
  apiPost: vi.fn(),
  apiPut: vi.fn(),
  apiDel: vi.fn(),
  authFetch: vi.fn(),
}));

const NIGHTLY = {
  id: 'nightly-etl',
  cron: '0 3 * * *',
  timezone: '+08:00',
  enabled: true,
  kind: 'dag',
  target: 'etl-demo',
  params: {},
  overlap: 'skip',
  node_id: 'n1',
  last_run: {
    schedule_id: 'nightly-etl',
    kind: 'dag',
    target: 'etl-demo',
    scheduled_for_ms: 1900000000000,
    fired_at_ms: 1900000001000,
    execution_id: 'dag-nightly-etl-1900000000000',
    status: 'fired',
    error: null,
    missed: false,
  },
  next_run: 1900868400000,
};
/// 停用 + invalid cron 的条目：last_run/next_run 均为 null，必须渲染「—」。
const PARKED = {
  id: 'parked',
  cron: 'not-a-cron',
  enabled: false,
  kind: 'agent',
  target: 'reviewer',
  overlap: 'allow',
  node_id: null,
  last_run: null,
  next_run: null,
};

const RUNS = [
  { schedule_id: 'nightly-etl', scheduled_for_ms: 1900000000000, fired_at_ms: 1900000001000, execution_id: 'dag-nightly-etl-1900000000000', status: 'fired', error: null },
  { schedule_id: 'nightly-etl', scheduled_for_ms: 1899913600000, fired_at_ms: 1899913600000, execution_id: null, status: 'missed', error: null },
  { schedule_id: 'nightly-etl', scheduled_for_ms: 1899827200000, fired_at_ms: 1899827200500, execution_id: null, status: 'error', error: '节点全部离线' },
];

beforeEach(() => {
  apiGetMock.mockReset().mockImplementation((path) => Promise.resolve(
    path === '/api/schedules'
      ? { schedules: [NIGHTLY, PARKED], scan_interval_secs: 15 }
      : { runs: RUNS },
  ));
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

/// antd 6 Button 给两字中文插空格（「刷 新」），按 role + 去空白匹配。
const findButton = (txt) => screen.getAllByRole('button')
  .find((b) => (b.textContent || '').replace(/\s+/g, '') === txt);

describe('SchedulePanel', () => {
  it('lists the schedules.json definitions with last/next fire', async () => {
    render(<SchedulePanel onNotice={vi.fn()} />);
    expect(apiGetMock).toHaveBeenCalledWith('/api/schedules');
    // 只读事实源提示 + 扫描间隔。
    expect(await screen.findByText('定时任务由控制面 schedules.json 定义，本页只读（每 15 秒扫描一次）')).toBeTruthy();
    const row = screen.getByText('nightly-etl').closest('tr');
    expect(within(row).getByText('0 3 * * *')).toBeTruthy();
    expect(within(row).getByText('启用')).toBeTruthy();
    expect(within(row).getByText('DAG')).toBeTruthy();
    expect(within(row).getByText('etl-demo')).toBeTruthy();
    expect(within(row).getByText('跳过重叠')).toBeTruthy();
    expect(within(row).getByText('n1')).toBeTruthy();
    expect(within(row).getByText('已触发')).toBeTruthy(); // last_run 的台账 status
    // invalid cron / 从未触发：null 时间渲染「—」。
    const parked = screen.getByText('parked').closest('tr');
    expect(within(parked).getAllByText('—').length).toBeGreaterThanOrEqual(2);
    expect(within(parked).getByText('停用')).toBeTruthy();
    expect(within(parked).getByText('允许重叠')).toBeTruthy();
  });

  it('shows the empty state when schedules.json declares nothing', async () => {
    apiGetMock.mockResolvedValue({ schedules: [], scan_interval_secs: null });
    render(<SchedulePanel onNotice={vi.fn()} />);
    expect(await screen.findByText('暂无定时任务')).toBeTruthy();
    // scan_interval_secs 为 null（服务端默认）时提示不拼扫描间隔。
    expect(screen.getByText('定时任务由控制面 schedules.json 定义，本页只读')).toBeTruthy();
  });

  it('surfaces a failed list load through onNotice(err)', async () => {
    apiGetMock.mockRejectedValue(Object.assign(new Error('HTTP 500'), { status: 500 }));
    const onNotice = vi.fn();
    render(<SchedulePanel onNotice={onNotice} />);
    await waitFor(() => expect(onNotice).toHaveBeenCalledWith(err('HTTP 500')));
  });

  it('opens the fire-history drawer with all three ledger statuses', async () => {
    render(<SchedulePanel onNotice={vi.fn()} />);
    await screen.findByText('nightly-etl');
    await act(async () => { fireEvent.click(findButton('触发历史')); });
    await waitFor(() => expect(apiGetMock).toHaveBeenCalledWith('/api/schedules/nightly-etl/runs?limit=50'));
    // 主表的 last_run 也有一颗「已触发」，断言一律圈定在 Drawer 的 portal 内
    // （antd 6 的内容容器是 .ant-drawer-body，旧 .ant-drawer-content 已更名）。
    const drawerEl = await waitFor(() => {
      const el = document.querySelector('.ant-drawer-body');
      expect(el).toBeTruthy();
      return el;
    });
    const drawer = within(drawerEl);
    expect(await drawer.findByText('已触发')).toBeTruthy();
    expect(drawer.getByText('已错过')).toBeTruthy();
    expect(drawer.getByText('失败')).toBeTruthy();
    expect(drawer.getByText('节点全部离线')).toBeTruthy(); // error 行的失败原因
    expect(drawer.getByText('dag-nightly-etl-1900000000000')).toBeTruthy();
    // missed 行没有 execution_id / error：两格都渲染「—」而不是空白。
    const missedRow = drawer.getByText('已错过').closest('tr');
    expect(within(missedRow).getAllByText('—').length).toBe(2);
  });
});
