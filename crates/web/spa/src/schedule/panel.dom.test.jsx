// @vitest-environment jsdom
// schedule/panel.dom.test.jsx —「调度」页 DOM 契约：定义列表（GET
// /api/schedules）、admin CRUD（POST/PUT/PATCH/DELETE + 手动 fire）、空态、
// 错误通知、新建表单简化（隐藏 ID/时区 + params 按类型单键分流）与触发历史
// Drawer（GET /api/schedules/:id/runs）的三种台账 status。只断言可观测
// DOM；api.js 模块级 mock，sse.js 不涉及（本页无事件流）。

import '../test/setup-dom.js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { err } from '../notice.js';
import { SchedulePanel } from './panel.jsx';

const { apiGetMock, apiPostMock, apiPutMock, apiPatchMock, apiDelMock } = vi.hoisted(() => ({
  apiGetMock: vi.fn(),
  apiPostMock: vi.fn(),
  apiPutMock: vi.fn(),
  apiPatchMock: vi.fn(),
  apiDelMock: vi.fn(),
}));
vi.mock('../api.js', () => ({
  apiGet: apiGetMock,
  apiPost: apiPostMock,
  apiPut: apiPutMock,
  apiPatch: apiPatchMock,
  apiDel: apiDelMock,
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
  apiPostMock.mockReset().mockResolvedValue({ ok: true, id: 'schedule-new' });
  apiPutMock.mockReset().mockResolvedValue({ ok: true });
  apiPatchMock.mockReset().mockResolvedValue({ ok: true });
  apiDelMock.mockReset().mockResolvedValue({ ok: true });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

/// antd 6 Button 给两字中文插空格（「刷 新」），按 role + 去空白匹配。
const findButton = (txt, root = document) => Array.from(root.querySelectorAll('button'))
  .find((b) => (b.textContent || '').replace(/\s+/g, '') === txt);

/// Open an antd Select and pick the dropdown option with the exact label —
/// options render in a body-level portal, so the search scopes to
/// .ant-select-item-option（同 team.dom.test.jsx 的范式）。
const pickSelectOption = async (selectEl, label) => {
  await act(async () => { fireEvent.mouseDown(selectEl); });
  const option = await waitFor(() => {
    const hit = [...document.querySelectorAll('.ant-select-item-option')]
      .find((o) => o.getAttribute('title') === label || o.textContent === label);
    expect(hit).toBeTruthy();
    return hit;
  });
  await act(async () => { fireEvent.click(option); });
  return option;
};

describe('SchedulePanel', () => {
  it('lists the store-backed definitions with last/next fire', async () => {
    render(<SchedulePanel onNotice={vi.fn()} />);
    expect(apiGetMock).toHaveBeenCalledWith('/api/schedules');
    const row = (await screen.findByText('nightly-etl')).closest('tr');
    // 种子语义提示已随页面简化整条移除。
    expect(screen.queryByText(/schedules\.json 仅作首次导入种子/)).toBeNull();
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
    // 行操作五件套。
    expect(within(row).getByText('触发历史')).toBeTruthy();
    expect(within(row).getByText('立即触发')).toBeTruthy();
    expect(within(row).getByText('编辑')).toBeTruthy();
    expect(within(row).getByText('停用')).toBeTruthy();
    expect(within(row).getByText('删除')).toBeTruthy();
  });

  it('shows the empty state when the store has no definitions', async () => {
    apiGetMock.mockResolvedValue({ schedules: [], scan_interval_secs: null });
    render(<SchedulePanel onNotice={vi.fn()} />);
    expect(await screen.findByText('暂无定时任务')).toBeTruthy();
    expect(screen.queryByText(/schedules\.json 仅作首次导入种子/)).toBeNull();
    expect(findButton('新建任务')).toBeTruthy();
  });

  it('surfaces a failed list load through onNotice(err)', async () => {
    apiGetMock.mockRejectedValue(Object.assign(new Error('HTTP 500'), { status: 500 }));
    const onNotice = vi.fn();
    render(<SchedulePanel onNotice={onNotice} />);
    await waitFor(() => expect(onNotice).toHaveBeenCalledWith(err('HTTP 500')));
  });

  it('creates a schedule: id/timezone hidden, params land in prompt, +08:00 pinned', async () => {
    render(<SchedulePanel onNotice={vi.fn()} />);
    await screen.findByText('nightly-etl');
    await act(async () => { fireEvent.click(findButton('新建任务')); });
    const modal = await waitFor(() => {
      const el = document.querySelector('.ant-modal');
      expect(el).toBeTruthy();
      return el;
    });
    // 新建隐藏 ID 与时区：id 由后端生成 schedule-<ULID>，时区固定 +08:00。
    expect(within(modal).queryByText('ID')).toBeNull();
    expect(within(modal).queryByText('时区')).toBeNull();
    await act(async () => {
      fireEvent.change(within(modal).getAllByPlaceholderText('0 3 * * *')[0], { target: { value: '*/5 * * * *' } });
    });
    // 目标必填：直接保存先被 antd 拦下（不发请求）。
    await act(async () => { fireEvent.click(findButton('保存', modal)); });
    await waitFor(() => expect(apiPostMock).not.toHaveBeenCalled());
    // 补齐目标与提示词后保存 → POST /api/schedules，成功通知 + 重新拉取列表。
    await act(async () => {
      fireEvent.change(within(modal).getByLabelText('schedule_target'), { target: { value: 'act' } });
      fireEvent.change(within(modal).getByLabelText('schedule_params'), { target: { value: '每日巡检' } });
    });
    await act(async () => { fireEvent.click(findButton('保存', modal)); });
    await waitFor(() => expect(apiPostMock).toHaveBeenCalledTimes(1));
    const [path, body] = apiPostMock.mock.calls[0];
    expect(path).toBe('/api/schedules');
    expect(body.id).toBeUndefined(); // 后端自动生成 schedule-<ULID>
    expect(body.timezone).toBe('+08:00');
    expect(body.cron).toBe('*/5 * * * *');
    expect(body.kind).toBe('agent');
    expect(body.params).toEqual({ prompt: '每日巡检' });
    expect(body.overlap).toBe('skip');
    expect(body.enabled).toBe(true);
  });

  it('creates a dag schedule: params become command-line args', async () => {
    render(<SchedulePanel onNotice={vi.fn()} />);
    await screen.findByText('nightly-etl');
    await act(async () => { fireEvent.click(findButton('新建任务')); });
    const modal = await waitFor(() => {
      const el = document.querySelector('.ant-modal');
      expect(el).toBeTruthy();
      return el;
    });
    // kind 下拉收敛为后端五种；切到 DAG（antd Select 需真开下拉再点选项）。
    const kindSelect = within(modal).getByLabelText('类型').closest('.ant-select');
    await pickSelectOption(kindSelect, 'DAG');
    // 切换后 params 文案随之变为「命令行参数」。
    expect(within(modal).getByText('命令行参数')).toBeTruthy();
    await act(async () => {
      fireEvent.change(within(modal).getAllByPlaceholderText('0 3 * * *')[0], { target: { value: '0 3 * * *' } });
      fireEvent.change(within(modal).getByLabelText('schedule_target'), { target: { value: 'etl-demo' } });
      fireEvent.change(within(modal).getByLabelText('schedule_params'), { target: { value: '--date {{now-1d:%Y-%m-%d}}' } });
    });
    await act(async () => { fireEvent.click(findButton('保存', modal)); });
    await waitFor(() => expect(apiPostMock).toHaveBeenCalledTimes(1));
    const [, body] = apiPostMock.mock.calls[0];
    expect(body.kind).toBe('dag');
    expect(body.params).toEqual({ args: '--date {{now-1d:%Y-%m-%d}}' });
    expect(body.timezone).toBe('+08:00');
  });

  it('edits a schedule through the editor modal (PUT keeps the id)', async () => {
    render(<SchedulePanel onNotice={vi.fn()} />);
    await screen.findByText('nightly-etl');
    const row = screen.getByText('nightly-etl').closest('tr');
    await act(async () => { fireEvent.click(within(row).getByText('编辑')); });
    const modal = await waitFor(() => {
      const el = document.querySelector('.ant-modal');
      expect(el).toBeTruthy();
      return el;
    });
    expect(within(modal).getAllByText('编辑定时任务 nightly-etl').length).toBeGreaterThanOrEqual(1);
    await waitFor(() => {
      // 回填：cron 与目标已在表单里。
      expect(within(modal).getAllByDisplayValue('0 3 * * *').length).toBeGreaterThan(0);
    });
    // 编辑保留 ID 与时区两字段（id 是主键、时区可改），并回填 +08:00。
    expect(within(modal).getByText('ID')).toBeTruthy();
    expect(within(modal).getAllByDisplayValue('+08:00').length).toBeGreaterThan(0);
    await act(async () => { fireEvent.click(findButton('保存', modal)); });
    await waitFor(() => expect(apiPutMock).toHaveBeenCalledTimes(1));
    const [path, body] = apiPutMock.mock.calls[0];
    expect(path).toBe('/api/schedules/nightly-etl');
    expect(body.id).toBe('nightly-etl');
    expect(body.target).toBe('etl-demo');
    expect(body.timezone).toBe('+08:00');
    expect(body.params).toEqual({}); // dag 记录 params 为空，编辑不凭空造键
  });

  it('edits merge params: the other keys survive an objective change', async () => {
    apiGetMock.mockResolvedValue({
      schedules: [{
        id: 'brain-job', cron: '0 3 * * *', timezone: '+08:00', enabled: true,
        kind: 'brain', target: 'plan-x',
        params: { objective: 'cut a plan', mode: 'fixed', inputs: { repo: 'opencoder' } },
        overlap: 'skip', node_id: null, last_run: null, next_run: null,
      }],
      scan_interval_secs: null,
    });
    render(<SchedulePanel onNotice={vi.fn()} />);
    await screen.findByText('brain-job');
    const row = screen.getByText('brain-job').closest('tr');
    await act(async () => { fireEvent.click(within(row).getByText('编辑')); });
    const modal = await waitFor(() => {
      const el = document.querySelector('.ant-modal');
      expect(el).toBeTruthy();
      return el;
    });
    // brain 的 params 是必填 objective，已按记录回填。
    const paramsArea = within(modal).getByLabelText('schedule_params');
    expect(paramsArea.value).toBe('cut a plan');
    await act(async () => {
      fireEvent.change(paramsArea, { target: { value: 'cut a better plan' } });
    });
    await act(async () => { fireEvent.click(findButton('保存', modal)); });
    await waitFor(() => expect(apiPutMock).toHaveBeenCalledTimes(1));
    const [path, body] = apiPutMock.mock.calls[0];
    expect(path).toBe('/api/schedules/brain-job');
    // objective 更新，mode/inputs 等其余键原样保留。
    expect(body.params).toEqual({
      objective: 'cut a better plan',
      mode: 'fixed',
      inputs: { repo: 'opencoder' },
    });
  });

  it('toggles enable via PATCH and deletes via DELETE', async () => {
    const onNotice = vi.fn();
    render(<SchedulePanel onNotice={onNotice} />);
    await screen.findByText('nightly-etl');
    await act(async () => { fireEvent.click(findButton('停用')); });
    await waitFor(() => expect(apiPatchMock).toHaveBeenCalledWith('/api/schedules/nightly-etl', { enabled: false }));
    // 等列表刷新落定后再走删除（避免点到骨架行）。
    await waitFor(() => expect(findButton('停用')).toBeTruthy());
    await act(async () => { fireEvent.click(findButton('删除')); });
    // Popconfirm 二次确认。
    fireEvent.click(await screen.findByText('确认删除'));
    await waitFor(() => expect(apiDelMock).toHaveBeenCalledWith('/api/schedules/nightly-etl'));
    await waitFor(() => expect(onNotice).toHaveBeenCalledWith({ type: 'success', text: '定时任务已删除（触发历史保留）' }));
  });

  it('manually fires a schedule with a confirm popover', async () => {
    render(<SchedulePanel onNotice={vi.fn()} />);
    await screen.findByText('nightly-etl');
    await act(async () => { fireEvent.click(findButton('立即触发')); });
    fireEvent.click(await screen.findByText('确认触发'));
    await waitFor(() => expect(apiPostMock).toHaveBeenCalledWith('/api/schedules/nightly-etl/run'));
  });

  it('surfaces a failed save through onNotice(err)', async () => {
    apiPostMock.mockRejectedValue(Object.assign(new Error('HTTP 409'), { status: 409 }));
    const onNotice = vi.fn();
    render(<SchedulePanel onNotice={onNotice} />);
    await screen.findByText('nightly-etl');
    await act(async () => { fireEvent.click(findButton('新建任务')); });
    const modal = await waitFor(() => {
      const el = document.querySelector('.ant-modal');
      expect(el).toBeTruthy();
      return el;
    });
    await act(async () => {
      fireEvent.change(within(modal).getAllByPlaceholderText('0 3 * * *')[0], { target: { value: '*/5 * * * *' } });
      fireEvent.change(within(modal).getByLabelText('schedule_target'), { target: { value: 'act' } });
    });
    await act(async () => { fireEvent.click(findButton('保存', modal)); });
    await waitFor(() => expect(onNotice).toHaveBeenCalledWith(err('保存定时任务失败: HTTP 409')));
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
