// @vitest-environment jsdom
// tableLoading.dom.test.jsx — 表格 loading 约定在真实 antd Table 上的守卫。
//
// 断言只看可观测 DOM：antd 6 的遮罩信号是 Spin 根节点上的 `.ant-spin-spinning`
// （antd 5 的 `.ant-spin-blur` 类已经没了），它让 `.ant-spin-container` 变成
// opacity .5 + pointer-events: none —— 行内链接/按钮当场点不动。所以「没遮罩」
// == 表格 wrapper 内没有 spinning 根节点，「可点」== 真的点开东西。
//
// 全程假定时器：Spin 的 delay 是 setTimeout，而 jsdom 里首屏 render 能跑几百
// 毫秒同步 JS，真实定时器会在 fetch 微任务之前到期，把遮罩状态搞乱（浏览器里
// 微任务永远赢，不存在这个问题）。假定时器下时钟只由测试推进，断言才稳定。
//
// api.js 走模块 mock（相对路径写法不同，但都解析到 src/api.js）；
// fleet/detail.jsx 只替掉 ExecutionDetail，避免把整棵详情树拖进来。

import '../test/setup-dom.js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';

const { apiGetMock, apiPostMock, apiPutMock, apiDelMock } = vi.hoisted(() => ({
  apiGetMock: vi.fn(),
  apiPostMock: vi.fn(),
  apiPutMock: vi.fn(),
  apiDelMock: vi.fn(),
}));
vi.mock('../api.js', () => ({
  apiGet: apiGetMock,
  apiPost: apiPostMock,
  apiPut: apiPutMock,
  apiDel: apiDelMock,
}));
vi.mock('../fleet/detail.jsx', () => ({
  ExecutionDetail: ({ id }) => <div>execution-detail:{id}</div>,
}));

import { err } from '../notice.js';
import { SPIN_DELAY_MS } from './tableLoading.js';
import { ExecutionsPanel } from '../fleet/executions.jsx';
import { FleetNodesPanel } from '../fleet/nodes.jsx';
import { FleetTeamsPanel } from '../fleet/teams.jsx';

const NODE = {
  id: 'n1', name: 'worker', online: true, kinds: ['agent'], maintenance_agent_id: 'maintainer-n1',
  snapshot: { ready: true, cpu_capacity: 2, active_agent_loops: 3, max_runs: 4, active_runs: 1, pending_runs: 0, queue_order: 'fifo' },
};
const TEAMS = { teams: [{ name: 'release', captain: 'lead', members: [{ id: 'lead', agent: 'act', role: '交付' }] }] };
/// 团队页一次 load 并发拉三个接口，兑现时给一个三种读法都成立的载荷。
const TEAM_PAGE = { ...TEAMS, nodes: [NODE], agents: [{ name: 'act' }] };
const execution = (id, created_at = 10) => ({ id, kind: 'agent', node_id: 'n1', status: 'done', created_at });

/// 表格是否被遮罩：wrapper 内出现 spinning 的 Spin 根节点即为遮罩。
const isMasked = () => !!document.querySelector('.ant-table-wrapper .ant-spin-spinning');

/// antd 6 Button 给两字中文插空格（「刷 新」），按 role + 去空白匹配。
const findButton = (txt) => screen.getAllByRole('button')
  .find((b) => (b.textContent || '').replace(/\s+/g, '') === txt);

/// 泵微任务：假定时器下不能用 waitFor / findBy（RTL 会去动 jest 的时钟）。
const flush = async (rounds = 6) => {
  for (let i = 0; i < rounds; i += 1) {
    await act(async () => { await Promise.resolve(); });
  }
};

/// 推进时钟并把 React 的更新一起冲掉。
const advance = async (ms) => { await act(async () => { vi.advanceTimersByTime(ms); }); };

/// 由测试手动兑现的请求（挂在 in-flight 状态，用来观察遮罩）。
const deferred = () => {
  let resolve;
  const promise = new Promise((ok) => { resolve = ok; });
  return { promise, resolve };
};

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  cleanup();
  vi.resetAllMocks();
  vi.useRealTimers();
});

describe('table loading convention in the DOM', () => {
  it('keeps the execution index unmasked and clickable while 加载更早的执行 appends', async () => {
    const append = deferred();
    apiGetMock.mockImplementation((path) => {
      if (path === '/api/nodes') {
        return Promise.resolve({ nodes: [NODE] });
      }
      if (String(path).includes('cursor_created_at')) {
        return append.promise;
      }
      return Promise.resolve({ executions: [execution('agent-new')], next_cursor: { created_at: 9, id: 'agent-next' } });
    });
    render(<ExecutionsPanel onNotice={vi.fn()} />);
    await flush();
    expect(screen.getByText('agent-new')).toBeTruthy();
    expect(isMasked()).toBe(false);

    fireEvent.click(screen.getByText('加载更早的执行'));
    await flush();
    expect(apiGetMock)
      .toHaveBeenCalledWith('/api/executions?limit=50&cursor_created_at=9&cursor_id=agent-next');
    // append 期间：翻页按钮自己转，表格不遮罩 —— 遮罩会让 ID 链接点不动，
    // 而且和按钮的 spinner 撞成两个。
    expect(findButton('加载更早的执行').className).toContain('ant-btn-loading');
    expect(isMasked()).toBe(false);
    await advance(SPIN_DELAY_MS * 2);
    expect(isMasked()).toBe(false);
    fireEvent.click(screen.getByText('agent-new'));
    expect(screen.getByText('execution-detail:agent-new')).toBeTruthy();

    await act(async () => { append.resolve({ executions: [execution('agent-old', 8)] }); });
    await flush();
    expect(screen.getByText('agent-old')).toBeTruthy();
    expect(screen.getByText('agent-new')).toBeTruthy();
    expect(isMasked()).toBe(false);
  });

  it('never masks the node table on the silent 3s poll, so row actions stay clickable', async () => {
    apiGetMock.mockResolvedValue({ nodes: [NODE] });
    render(<FleetNodesPanel onNotice={vi.fn()} />);
    await flush();
    expect(screen.getByText('worker')).toBeTruthy();
    expect(isMasked()).toBe(false);

    // 轮询请求挂住：静默刷新即使慢也不该遮罩（否则每 3 秒锁一次行内按钮）。
    apiGetMock.mockImplementation(() => new Promise(() => {}));
    await advance(3000);
    await flush();
    expect(apiGetMock).toHaveBeenCalledTimes(2); // /api/nodes：首屏 1 次 + 静默轮询 1 次
    expect(isMasked()).toBe(false);
    expect(document.querySelector('.ant-spin-spinning')).toBeNull();

    const maintain = findButton('维护节点');
    expect(maintain.disabled).toBe(false);
    await act(async () => { fireEvent.click(maintain); });
    expect(screen.getByText(/节点维护 · worker/)).toBeTruthy();
  });

  it('clears the mask after a rejected fetch instead of spinning forever', async () => {
    const onNotice = vi.fn();
    apiGetMock.mockRejectedValue(new Error('nodes unavailable'));
    render(<FleetNodesPanel onNotice={onNotice} />);
    await flush();
    expect(onNotice).toHaveBeenCalledWith(err(expect.stringContaining('nodes unavailable')));
    // 越过 SPIN_DELAY_MS：若 finally 没有清 loading，遮罩此刻就会出现。
    await advance(SPIN_DELAY_MS * 2);
    await flush();
    expect(isMasked()).toBe(false);
    expect(document.querySelector('.ant-spin-spinning')).toBeNull();
  });

  it('does not claim 暂无 Opencoder 节点 while the first fetch is still unknown', async () => {
    const first = deferred();
    apiGetMock.mockImplementation(() => first.promise);
    render(<FleetNodesPanel onNotice={vi.fn()} />);
    await flush();
    // dataSource 交回 undefined → antd 抑制空态占位；[] 会一边拉取一边撒谎。
    expect(document.body.textContent).not.toContain('暂无 Opencoder 节点');
    await act(async () => { first.resolve({ nodes: [] }); });
    await flush();
    expect(screen.getByText('暂无 Opencoder 节点')).toBeTruthy();
  });

  it('hides a refresh shorter than the spin delay and only masks a still-pending one', async () => {
    apiGetMock.mockImplementation((path) => Promise.resolve(path === '/api/teams' ? TEAMS
      : (path === '/api/nodes' ? { nodes: [NODE] } : { agents: [{ name: 'act' }] })));
    render(<FleetTeamsPanel onNotice={vi.fn()} />);
    await flush();
    expect(screen.getByText('release')).toBeTruthy();
    expect(isMasked()).toBe(false);

    const refresh = deferred();
    apiGetMock.mockImplementation(() => refresh.promise);
    await act(async () => { fireEvent.click(findButton('刷新')); });
    await advance(SPIN_DELAY_MS - 50);
    expect(isMasked()).toBe(false); // 快刷新：一帧遮罩都不给
    await advance(50);
    expect(isMasked()).toBe(true); // 真的还在拉：该遮就遮
    await act(async () => { refresh.resolve(TEAM_PAGE); });
    await flush();
    expect(isMasked()).toBe(false);
    expect(screen.getByText('release')).toBeTruthy();
  });
});
