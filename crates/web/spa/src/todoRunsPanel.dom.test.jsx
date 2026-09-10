// @vitest-environment jsdom
// todoRunsPanel.dom.test.jsx — TODO「运行」视图两张表 loading 语义的守卫。
//
// 断言只看可观测 DOM：antd 6 的遮罩信号是 Spin 根节点上的 `.ant-spin-spinning`
// （v5 的 `.ant-spin-blur` 已无），它让 `.ant-spin-container` 变成 opacity .5 +
// pointer-events: none —— 左侧行点击（本面板唯一的主交互）和详情里的中断/
// 恢复/取消按钮当场全部点不动。
//
// 两张表可能同时在场，遮罩断言必须作用域化。实测 antd 6 把 Table 的
// className 落在 `.ant-table-wrapper` 根节点上（es/table/InternalTable.js 的
// wrappercls），Spin 是该 div 的子孙，所以 `.oc-todo-runs .ant-spin-spinning`
// 与 `.oc-todo-items .ant-spin-spinning` 各自只命中自己那张表。
//
// 全程假定时器：Spin 的 delay 与 3s 轮询都是 setTimeout/setInterval，jsdom 里
// 真实定时器会抢在 fetch 微任务之前到期把遮罩状态搞乱；绝不混用 waitFor/findBy。
// api.js 模块级 mock；sse.js 与 fleet/detail.jsx 用替身，不把整棵详情树拖进来。

import './test/setup-dom.js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';

const { apiGetMock, apiPostMock, apiPutMock, apiDelMock } = vi.hoisted(() => ({
  apiGetMock: vi.fn(),
  apiPostMock: vi.fn(),
  apiPutMock: vi.fn(),
  apiDelMock: vi.fn(),
}));
vi.mock('./api.js', () => ({
  apiGet: apiGetMock,
  apiPost: apiPostMock,
  apiPut: apiPutMock,
  apiDel: apiDelMock,
}));
vi.mock('./sse.js', () => ({ openStream: vi.fn(() => ({ abort: () => {} })) }));
vi.mock('./fleet/detail.jsx', () => ({ ExecutionDetail: ({ id }) => <div>execution-detail:{id}</div> }));

import { SPIN_DELAY_MS } from './ui/tableLoading.js';
import { TodoRunsPanel } from './todoRunsPanel.jsx';

/// 面板内部的轮询间隔（未导出，这里与源码保持一致）。
const POLL_MS = 3000;
const LIST_PATH = '/api/todo/workflows?limit=50';
const DETAIL_PATH = '/api/todo/workflows/todos-1';
const WORKFLOWS = {
  workflows: [{
    id: 'todos-1', status: 'running', execution_status: 'running',
    execution_created_at: 1, node_id: 'node-a', updated_at: 2,
  }],
};
const DETAIL = {
  workflow: { id: 'todos-1', status: 'running' },
  items: [{ todo_id: 't1', status: 'running', attempt: 1, active_session_id: 'ses-1' }],
};

/// 工作流表是否被遮罩（作用域化：只认这张表的 wrapper 子树）。
const outerMasked = () => !!document.querySelector('.oc-todo-runs .ant-spin-spinning');
/// TODO 项表是否被遮罩。
const innerMasked = () => !!document.querySelector('.oc-todo-items .ant-spin-spinning');

/// 「暂无数据」占位：emptyText 为 null 时 rc-table 仍留一条空的
/// `.ant-table-placeholder` tr，真正对用户撒谎的是里面的 `.ant-empty` 组件。
const emptyLies = (scope) => !!document.querySelector(`${scope} .ant-empty`);

/// 行文本按表作用域化取：详情卡标题也会渲染 `todos-1…`，全局 getByText 会撞车。
const rowText = (scope) => Array.from(document.querySelectorAll(`${scope} tbody tr.ant-table-row`))
  .map((r) => r.textContent || '').join('|');

/// antd 6 Button 给两字中文插空格（「刷 新」），按 role + 去空白匹配。
const findButton = (txt) => screen.getAllByRole('button')
  .find((b) => (b.textContent || '').replace(/\s+/g, '') === txt);

/// scroll.x 打开后 rc-table 会在 tbody 首行插一条 measure-row，取数据行要带类名。
const clickFirstRow = (scope) => {
  const row = document.querySelector(`${scope} tbody tr.ant-table-row`);
  expect(row).toBeTruthy();
  fireEvent.click(row);
  return row;
};

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

const installApi = () => {
  apiGetMock.mockReset().mockImplementation((path) => Promise.resolve(path === DETAIL_PATH ? DETAIL : WORKFLOWS));
  apiPostMock.mockReset().mockResolvedValue({ ok: true });
  apiPutMock.mockReset().mockResolvedValue({ ok: true });
  apiDelMock.mockReset().mockResolvedValue({ ok: true });
};

beforeEach(() => {
  vi.useFakeTimers();
  installApi();
});

afterEach(() => {
  cleanup();
  // 不用 vi.resetAllMocks()：它会连 sse.js 替身的实现一起清掉，openStream 返回
  // undefined，EventsFeed 的 effect cleanup 就在 handle.abort() 上炸。
  // api mock 由 beforeEach 的 installApi() 逐个 mockReset。
  vi.useRealTimers();
});

describe('TodoRunsPanel 表格 loading 语义', () => {
  it('首屏在途时不宣称暂无数据，兑现后行照常渲染', async () => {
    const first = deferred();
    apiGetMock.mockImplementation((path) => (path === LIST_PATH ? first.promise : Promise.resolve(DETAIL)));
    render(<TodoRunsPanel onNotice={vi.fn()} />);
    await flush();
    // dataSource 交回 undefined → antd 抑制空态；[] 会一边拉取一边撒谎。
    expect(emptyLies('.oc-todo-runs')).toBe(false);
    expect(document.body.textContent).not.toContain('暂无');

    await act(async () => { first.resolve(WORKFLOWS); });
    await flush();
    expect(rowText('.oc-todo-runs')).toContain('todos-1');
    expect(outerMasked()).toBe(false);

    // 反向证明：数据真为空时占位确实会出现，上面的 false 才不是死选择器。
    apiGetMock.mockImplementation((path) => (path === LIST_PATH ? Promise.resolve({ workflows: [] }) : Promise.resolve(DETAIL)));
    await act(async () => { fireEvent.click(findButton('刷新')); });
    await flush();
    expect(emptyLies('.oc-todo-runs')).toBe(true);
  });

  it('首屏拉取越过 SPIN_DELAY_MS 才亮遮罩，兑现后遮罩消失（证明选择器是活的）', async () => {
    const first = deferred();
    apiGetMock.mockImplementation((path) => (path === LIST_PATH ? first.promise : Promise.resolve(DETAIL)));
    render(<TodoRunsPanel onNotice={vi.fn()} />);
    await flush();
    await advance(SPIN_DELAY_MS - 50);
    expect(outerMasked()).toBe(false); // 快拉取：一帧遮罩都不给
    await advance(50);
    // 真还在拉：该遮就遮。这一条 true 让其余 false 断言不可能因选择器失效而空过。
    expect(outerMasked()).toBe(true);
    expect(document.querySelector('.oc-todo-runs .ant-spin-container')).toBeTruthy();

    await act(async () => { first.resolve(WORKFLOWS); });
    await flush();
    expect(outerMasked()).toBe(false);
    expect(rowText('.oc-todo-runs')).toContain('todos-1');
  });

  it('点击行选中工作流；详情首拉在途时 TODO 项表同样不撒谎', async () => {
    const detailReq = deferred();
    apiGetMock.mockImplementation((path) => (path === LIST_PATH ? Promise.resolve(WORKFLOWS) : detailReq.promise));
    render(<TodoRunsPanel onNotice={vi.fn()} />);
    await flush();
    const row = clickFirstRow('.oc-todo-runs');
    await flush();

    expect(apiGetMock).toHaveBeenCalledWith(DETAIL_PATH);
    expect(row.className).toContain('oc-row-selected');
    expect(document.querySelector('.oc-todo-items')).toBeTruthy();
    // 详情不知道就是不知道：空态占位不得抢在数据前面出现。
    expect(emptyLies('.oc-todo-items')).toBe(false);
    expect(innerMasked()).toBe(false);
    // 越过 SPIN_DELAY_MS：首拉该遮就遮（详情里此刻还没有任何可点的东西）。
    // 这条 true 同时证明 `.oc-todo-items` 作用域选择器是活的。
    await advance(SPIN_DELAY_MS + 50);
    expect(innerMasked()).toBe(true);
    expect(outerMasked()).toBe(false);

    await act(async () => { detailReq.resolve(DETAIL); });
    await flush();
    expect(rowText('.oc-todo-items')).toContain('t1');
    expect(document.querySelectorAll('.oc-todo-items tbody tr.ant-table-row')).toHaveLength(1);
    expect(innerMasked()).toBe(false);
  });

  it('中断后的双表刷新静默：越过 SPIN_DELAY_MS 也不遮罩，行与按钮都还在', async () => {
    render(<TodoRunsPanel onNotice={vi.fn()} />);
    await flush();
    clickFirstRow('.oc-todo-runs');
    await flush();
    expect(rowText('.oc-todo-items')).toContain('t1');
    expect(outerMasked()).toBe(false);
    expect(innerMasked()).toBe(false);

    // 变更后的两个刷新都挂住：这一段在途时间就是「行点击 + 行内按钮被锁死」的窗口。
    const refreshList = deferred();
    const refreshDetail = deferred();
    apiGetMock.mockImplementation((path) => (path === LIST_PATH ? refreshList.promise : refreshDetail.promise));
    await act(async () => { fireEvent.click(findButton('中断（可恢复）')); });
    await flush();
    expect(apiPostMock).toHaveBeenCalledWith('/api/todo/workflows/todos-1/interrupt', {});
    expect(apiGetMock).toHaveBeenCalledWith(LIST_PATH);
    expect(apiGetMock).toHaveBeenCalledWith(DETAIL_PATH);

    await advance(SPIN_DELAY_MS * 4);
    await flush();
    expect(outerMasked()).toBe(false);
    expect(innerMasked()).toBe(false);
    expect(document.querySelector('.ant-spin-spinning')).toBeNull();
    // 数据未被抽空，用户下一步要点的按钮也没被 pointer-events: none 锁住。
    expect(rowText('.oc-todo-runs')).toContain('todos-1');
    expect(rowText('.oc-todo-items')).toContain('t1');
    expect(findButton('取消（终止）').disabled).toBe(false);

    await act(async () => { refreshList.resolve(WORKFLOWS); refreshDetail.resolve(DETAIL); });
    await flush();
    expect(rowText('.oc-todo-runs')).toContain('todos-1');
    expect(rowText('.oc-todo-items')).toContain('t1');
    expect(outerMasked()).toBe(false);
    expect(innerMasked()).toBe(false);
  });

  it('3s 轮询静默：挂住的 poll 不点着任何一张表的遮罩', async () => {
    render(<TodoRunsPanel onNotice={vi.fn()} />);
    await flush();
    clickFirstRow('.oc-todo-runs');
    await flush();
    expect(rowText('.oc-todo-runs')).toContain('todos-1');
    expect(rowText('.oc-todo-items')).toContain('t1');
    expect(apiGetMock.mock.calls.filter((c) => c[0] === LIST_PATH)).toHaveLength(1);

    // 轮询请求挂住：仍有 running 工作流时每 3s 一次，遮罩会把行点击锁死。
    const poll = deferred();
    apiGetMock.mockImplementation((path) => (path === LIST_PATH ? poll.promise : Promise.resolve(DETAIL)));
    await advance(POLL_MS);
    await flush();
    expect(apiGetMock.mock.calls.filter((c) => c[0] === LIST_PATH)).toHaveLength(2);
    await advance(SPIN_DELAY_MS * 4);
    expect(outerMasked()).toBe(false);
    expect(innerMasked()).toBe(false);
    expect(document.querySelector('.ant-spin-spinning')).toBeNull();

    await act(async () => { poll.resolve(WORKFLOWS); });
    await flush();
    expect(rowText('.oc-todo-runs')).toContain('todos-1');
    expect(outerMasked()).toBe(false);
    expect(innerMasked()).toBe(false);
  });
});
