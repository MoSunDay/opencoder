// @vitest-environment jsdom
// layered.dom.test.jsx — the BrainRunBody schema_version 4 branch: layered
// canvas, layer-barrier progress, layer decisions and the layered journal.
// It also guards the two neighbouring branches: a v3 run must never request
// /layered (byte-identical v3 rendering) and an unknown version must be an
// explicit error instead of a fallback.
import '../../../../test/setup-dom.js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { apiGet } from '../../../../api.js';
import { openStream } from '../../../../sse.js';
import { BrainRunBody } from '../../run.jsx';

vi.mock('../../../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPut: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../../../fleet/detail.jsx', () => ({ ExecutionView: ({ executionRef }) => <div data-testid="capability-detail">{executionRef.kind}:{executionRef.id}</div> }));
vi.mock('../../../../sse.js', () => ({ openStream: vi.fn(() => ({ abort() {} })) }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });

const operation = (layer, node, attempt, status, executionId, extra = {}) => ({
  operation_id: `brain-v4#l${layer}#${node}#a${attempt}`, layer, node_id: node, attempt,
  capability_id: `cap-${node}`, execution_kind: 'agent', execution_id: executionId,
  status, cancel_requested: false, ...extra,
});

const view = {
  schema_version: 4,
  run: { run_id: 'brain-v4', phase: 'waiting', layer: 2, generation: 4, last_event_seq: 12, summary: null, error: null, depth: 0, parent: null, total_layers: 3, created_at: 100, updated_at: 200 },
  plan: {
    title: '分层能力画布', objective: '按层交付画布与决策明细', todo: { id: 'todo-7' }, max_rounds: 32,
    nodes: [
      { node_id: 'n-fetch', title: '抓取仓库', capability_id: 'cap-n-fetch', retry: { max_attempts: 2 } },
      { node_id: 'n-api', title: 'API 影响面', capability_id: 'cap-n-api', retry: { max_attempts: 3 } },
      { node_id: 'n-ui', title: '前端画布', capability_id: 'cap-n-ui', retry: { max_attempts: 2 } },
      { node_id: 'n-verdict', title: '汇总裁决', capability_id: 'cap-n-verdict', retry: { max_attempts: 2 } },
    ],
    edges: [{ from: 'n-fetch', to: 'n-api' }, { from: 'n-fetch', to: 'n-ui' }, { from: 'n-api', to: 'n-verdict' }, { from: 'n-ui', to: 'n-verdict' }],
  },
  layers: [['n-fetch'], ['n-api', 'n-ui'], ['n-verdict']],
  operations: [
    operation(1, 'n-fetch', 1, 'done', 'exec-fetch-a1'),
    operation(2, 'n-api', 1, 'done', 'exec-api-a1'),
    operation(2, 'n-ui', 1, 'error', 'exec-ui-a1'),
    operation(2, 'n-ui', 2, 'running', 'exec-ui-a2', { cancel_requested: true }),
  ],
  events: [
    { seq: 4, layer: 2, event_type: 'operation_retry_scheduled', node_id: 'n-ui', attempt: 2, execution_id: 'exec-ui-a2', decision_summary: null, reason_summary: '前端画布失败，重试第 2 次', evidence_execution_ids: ['exec-ui-a1'], at_ms: 200 },
    { seq: 1, layer: 0, event_type: 'run_created', node_id: null, attempt: null, execution_id: null, decision_summary: 'dispatch_layer', reason_summary: null, evidence_execution_ids: [], at_ms: 100 },
  ],
  capabilities: [{ capability_id: 'cap-n-fetch', kind: 'agent', target: 'act-fetch', version: 1 }],
};

const round = {
  schema_version: 4, layer: 1, phase: 'dispatch_layer', reason: '抓取完成，进入并行层', evidence_execution_ids: ['exec-fetch-a1'],
  nodes: [{ node_id: 'n-fetch', title: '抓取仓库', capability_id: 'cap-n-fetch', status: 'done', attempt: 1, execution_id: 'exec-fetch-a1', execution_kind: 'agent', cancel_requested: false, summary: '仓库已抓取', inputs: { repo: { kind: 'root', name: 'repo' } } }],
};

/// Currently served v4 payload; tests overwrite it to move the run forward.
let served = view;
/// Currently served run snapshot (the base `/api/brain/runs/:id` body).
let snapshot = () => ({ schema_version: 4, run: view.run });

const paths = () => apiGet.mock.calls.map((call) => String(call[0]));

function route() {
  apiGet.mockImplementation(async (path) => {
    if (/\/layered\/rounds\/\d+$/.test(path)) return round;
    if (path.endsWith('/layered')) return served;
    if (path.endsWith('/view')) return served;
    return snapshot();
  });
}

const mount = () => render(<BrainRunBody id="brain-v4" />);

describe('分层能力计划', () => {
  it('渲染分层画布、层屏障、重试徽标与分层事件，并按需拉取层决策明细', async () => {
    route();
    mount();
    await waitFor(() => expect(document.querySelectorAll('.brain-layer-node')).toHaveLength(4));
    expect(paths()).toContain('/api/brain/runs/brain-v4/layered');
    expect(paths()).not.toContain('/api/brain/runs/brain-v4/view');

    // 画布：每层一列，正在决策的层高亮，重试与取消徽标来自最新尝试
    expect([...document.querySelectorAll('.brain-layer-node-title')].map((node) => node.textContent)).toEqual(['抓取仓库', 'API 影响面', '前端画布', '汇总裁决']);
    const active = document.querySelectorAll('.brain-layer-node--active');
    expect(active).toHaveLength(2);
    expect(active[0].textContent).toContain('API 影响面');
    expect(active[1].textContent).toContain('前端画布');
    const retried = document.querySelector('.brain-layer-node--running');
    expect(retried.textContent).toContain('尝试 2/2');
    expect(retried.textContent).toContain('重试 1 次');
    expect(retried.textContent).toContain('已请求取消');
    expect(document.querySelectorAll('.brain-layer-node--error')).toHaveLength(0); // 被替代的失败尝试不覆盖最新尝试
    // 边的可见性不依赖 ResizeObserver：声明式节点在首帧即可连线
    await waitFor(() => expect(document.querySelectorAll('.react-flow__edge')).toHaveLength(4));

    // 运行头部：版本、阶段、层屏障进度、TODO 与交付摘要
    expect(screen.getByText('分层能力计划')).toBeTruthy();
    expect(screen.getByText('等待执行')).toBeTruthy();
    expect(screen.getByText('层屏障 1/3')).toBeTruthy();
    expect(screen.getByText('TODO todo-7')).toBeTruthy();
    expect(screen.getByText(/已完成 1 \/ 共 3 层 · 已派发至第 2 层/)).toBeTruthy();
    expect(screen.getByText('分层能力画布')).toBeTruthy();

    // 层决策：当前决策层自动展开，未派发的层不请求明细
    await waitFor(() => expect(paths()).toContain('/api/brain/runs/brain-v4/layered/rounds/2'));
    expect(paths()).not.toContain('/api/brain/runs/brain-v4/layered/rounds/3');
    fireEvent.click(screen.getByRole('button', { name: /^层 1/ }));
    expect(await screen.findByText('抓取完成，进入并行层')).toBeTruthy();
    expect(paths()).toContain('/api/brain/runs/brain-v4/layered/rounds/1');
    expect(screen.getAllByText('本层决策：派发本层').length).toBeGreaterThan(0);
    expect(screen.queryByText('仓库已抓取')).toBeNull();
    expect(document.querySelector('[data-node="n-fetch"]').textContent).toContain('exec-fetch-a1');
    expect(document.querySelectorAll('[data-node="n-fetch"]')).toHaveLength(2);

    // 分层事件日志：类型、节点、尝试与缘由
    fireEvent.click(screen.getByRole('button', { name: /分层事件（2）/ }));
    expect(await screen.findByText('重试排期')).toBeTruthy();
    expect(screen.getByText('前端画布失败，重试第 2 次')).toBeTruthy();
    expect(screen.getByText('运行创建')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /本次运行关联能力（1）/ }));
    expect(await screen.findByText('act-fetch')).toBeTruthy();
  });

  it('v4 事件帧触发分层视图重取，运行推进后画布与屏障同步更新', async () => {
    served = { ...view, run: { ...view.run, layer: 1 }, operations: view.operations.slice(0, 1) };
    route();
    mount();
    await waitFor(() => expect(document.querySelectorAll('.brain-layer-node')).toHaveLength(4));
    expect(screen.getByText('层屏障 1/3')).toBeTruthy();
    expect(screen.getByText('已完成 1 / 共 3 层 · 已派发至第 1 层')).toBeTruthy();
    const before = apiGet.mock.calls.length;
    served = view;
    const frame = openStream.mock.calls[0][0].onFrame;
    await act(async () => frame({ seq: 13, event_type: 'layer_barrier_reached', data: {} }));
    await waitFor(() => expect(apiGet.mock.calls.length).toBeGreaterThan(before));
    await waitFor(() => expect(screen.getByText('已完成 1 / 共 3 层 · 已派发至第 2 层')).toBeTruthy());
    expect(document.querySelector('.brain-layer-node--running').textContent).toContain('重试 1 次');
    await act(async () => openStream.mock.calls[0][0].onStatus('closed'));
  });

  it('点击 step 复用能力运行组件，并可查看重试前执行', async () => {
    served = view; route(); mount();
    await waitFor(() => expect(document.querySelectorAll('.brain-layer-node')).toHaveLength(4));
    fireEvent.click(document.querySelector('[data-id="n-api"]'));
    expect(await screen.findByTestId('capability-detail')).toBeTruthy();
    expect(screen.getByTestId('capability-detail').textContent).toContain('agent:');
    expect(screen.getByRole('combobox', { name: '选择执行尝试' })).toBeTruthy();
  });

  it('没有事件流也不会冻结：分层视图每 3 秒轮询重取 /layered', async () => {
    served = { ...view, run: { ...view.run, layer: 1 }, operations: view.operations.slice(0, 1) };
    route();
    vi.useFakeTimers();
    try {
      mount();
      await act(async () => { await vi.advanceTimersByTimeAsync(0); });
      expect(paths().filter((path) => path.endsWith('/layered'))).toHaveLength(1);
      expect(document.body.textContent).not.toContain('重试 1 次');
      served = view;
      await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
      expect(paths().filter((path) => path.endsWith('/layered'))).toHaveLength(2);
      expect(document.body.textContent).toContain('重试 1 次');
      expect(screen.getByText('已完成 1 / 共 3 层 · 已派发至第 2 层')).toBeTruthy();
    } finally { vi.useRealTimers(); }
  });

  it.each([3, 5, '4'])('拒绝不支持的版本 %s', async (version) => {
    apiGet.mockResolvedValue({ schema_version: version, run: {} });
    render(<BrainRunBody id="brain-unsupported" />);
    await screen.findByText('运行缺少分层调度数据');
    expect(paths().every((path) => path.endsWith('/layered'))).toBe(true);
  });
  it('缺少运行时明确报错', async () => {
    apiGet.mockResolvedValue({ schema_version: 4 });
    render(<BrainRunBody id="brain-broken" />);
    await screen.findByText('运行缺少分层调度数据');
  });

  it('直接查询分层视图，不查询旧版运行快照', async () => {
    apiGet.mockImplementation(async (path) => {
      if (path.endsWith('/layered')) return view;
      const missing = new Error('not found'); missing.status = 404; throw missing;
    });
    mount();
    await waitFor(() => expect(document.querySelectorAll('.brain-layer-node')).toHaveLength(4));
    expect(paths()).toContain('/api/brain/runs/brain-v4/layered');
    expect(paths()).not.toContain('/api/brain/runs/brain-v4');
    expect(screen.getByText('层屏障 1/3')).toBeTruthy();
  });
});
