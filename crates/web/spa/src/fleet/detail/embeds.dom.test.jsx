// @vitest-environment jsdom
// embeds.dom.test.jsx — 执行明细内嵌运行视图的 DOM 守卫：
// - brain：BrainRunEmbed 复用工作台 BrainRunBody（步骤列表 + PlanCanvas +
//   Inspector），但绝不写 brain_run URL 参数（那是 BrainRunView 的职责，
//   明细抽屉的位置语义由抽屉自身承载）。
// - todos：TodoRunEmbed 拉分体 store 的 /api/todo/workflows/:id 渲染调度画布；
//   拉取失败（分体部署下 node 侧 store 不可见）时静默返回 null，明细里
//   workloads.jsx 的 TodoDetail 列表仍在下方回退。
import '../../test/setup-dom.js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { BrainRunEmbed } from './brainRun.jsx';
import { TodoRunEmbed } from './todoCanvas.jsx';
import { ExecutionView } from '../detail.jsx';
import { apiGet } from '../../api.js';

vi.mock('../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPut: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../sse.js', () => ({ openStream: vi.fn(() => ({ abort() {} })) }));

// 不用 vi.resetAllMocks()：它会连 sse.js 替身的实现一起清掉，openStream 返回
// undefined，事件流 effect 的 cleanup 就在 handle.abort() 上炸（同
// todoRunsPanel.dom.test.jsx 的做法）；这里用 clearAllMocks 只清调用记录，
// apiGet 的实现由每个测试自行 mockImplementation 重挂。
afterEach(() => { cleanup(); vi.clearAllMocks(); });

describe('执行明细内嵌运行视图', () => {
  it('brain 明细复用工作台运行主体，但不写 brain_run URL 参数', async () => {
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/brain/runs/brain-1') {
        return { objective: '发布里程碑', phase: 'completed', activation: 1, revision: 1, handled_revision: 1, updated_at: 1, total_instances: 1, error: '', input_requests: {}, deliverables: {},
          plan: { id: 'plan-1', version: 2, plan: { steps: [{ id: 's1', label: '第一步', action: { kind: 'agent' } }] } },
          groups: [], instances: [] };
      }
      if (String(path).startsWith('/api/brain/runs/brain-1/events-page')) return { events: [], more: false };
      return {};
    });
    render(<BrainRunEmbed id="brain-1" onNotice={vi.fn()} />);
    // 画布节点也渲染同名 label，断言作用域化到步骤列表。
    expect(await screen.findByText('第一步', { selector: '.brain-step-list strong' })).toBeTruthy();
    expect(document.querySelector('.brain-workspace')).toBeTruthy(); // PlanCanvas + Inspector 容器
    expect(apiGet).toHaveBeenCalledWith('/api/brain/runs/brain-1');
    // Embed 不得写 brain_run 参数（BrainRunView 才同步浏览器地址）。
    expect(window.location.search).toBe('');
  });

  it('todos 明细内嵌调度画布，点击节点联动 Inspector', async () => {
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/todo/workflows/todos-1') {
        return { workflow: { status: 'running', spec_json: { todos: [
          { id: 't1', title: '调研', agent: 'explore', depends_on: [], acceptance: { criteria: '对比' } },
          { id: 't2', title: '实现', agent: 'act', depends_on: ['t1'], acceptance: { criteria: '通过' } }] } },
          items: [{ todo_id: 't1', status: 'passed', attempt: 1, active_session_id: null, last_error: '' }] };
      }
      return {};
    });
    render(<TodoRunEmbed id="todos-1" />);
    await waitFor(() => expect(document.querySelector('.oc-todo-run-node[data-todo-id="t1"]')).toBeTruthy());
    expect(document.querySelector('.oc-todo-run-node[data-todo-id="t2"]')).toBeTruthy();
    expect(screen.getByText(/1\/2 已通过/)).toBeTruthy(); // 进度统计来自 items 投影
    fireEvent.click(document.querySelector('.oc-todo-run-node[data-todo-id="t2"]'));
    // 画布节点也渲染同名标题，Inspector 断言按容器作用域取。
    await waitFor(() => {
      const inspector = document.querySelector('.oc-todo-run-inspector');
      expect(inspector).toBeTruthy();
      expect(inspector.textContent).toContain('实现');
    });
  });

  it('todos 画布拉取失败时静默回退，TodoDetail 列表仍在', async () => {
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/executions/todos-x') {
        return { execution: { id: 'todos-x', kind: 'todos', status: 'done', created_at: 1 },
          request: { kind: 'todos', target: 'tpl/v1', input: {} },
          workflow: { workflow: { status: 'completed' }, items: [{ todo_id: 't1', status: 'done', attempt: 1 }] } };
      }
      if (path === '/api/todo/workflows/todos-x') throw new Error('node store unreachable'); // 分体部署 node 侧 store 不可见
      return {};
    });
    render(<ExecutionView executionRef={{ id: 'todos-x', kind: 'todos' }} onNotice={vi.fn()} />);
    // 「TODO 工作流」同时是 todos 的类型标签（头部 Descriptions），断言限定
    // 到 TodoDetail 回退块的标题节点。
    expect(await screen.findByText('TODO 工作流', { selector: 'h5' })).toBeTruthy(); // TodoDetail 回退仍在
    expect(screen.queryByText('TODO 调度画布')).toBeNull(); // 画布失败静默，不渲染
  });

  it('inline 模式（工作台 Inspector「执行过程」页）不挂过程视图，full 模式才挂', async () => {
    // brain 能力含 todos：todos 实例会在 Inspector 的 ~380px 窄列里以
    // mode="inline" 复用 ExecutionView——即使画布数据可拉，也不得挂
    // TodoRunCanvas/第二条 SSE（mode === 'full' 门控），轻量块照常渲染。
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/executions/todos-insp') {
        return { execution: { id: 'todos-insp', kind: 'todos', status: 'running', created_at: 1 },
          request: { kind: 'todos', target: 'tpl/v1', input: {} },
          workflow: { workflow: { status: 'running' }, items: [] } };
      }
      if (path === '/api/todo/workflows/todos-insp') {
        return { workflow: { status: 'running', spec_json: { todos: [
          { id: 't1', title: '内联步骤', agent: 'act', depends_on: [], acceptance: { criteria: '通过' } }] } }, items: [] };
      }
      return {};
    });
    const inline = render(<ExecutionView executionRef={{ id: 'todos-insp', kind: 'todos' }} mode="inline" managed onNotice={vi.fn()} />);
    expect(await inline.findByText('TODO 工作流', { selector: 'h5' })).toBeTruthy(); // TodoDetail 轻量块仍在
    expect(inline.queryByText('TODO 调度画布')).toBeNull(); // 窄列不挂画布
    expect(apiGet.mock.calls.filter(([path]) => path === '/api/todo/workflows/todos-insp')).toHaveLength(0); // 门控：inline 连画布数据都不拉
    inline.unmount();

    const full = render(<ExecutionView executionRef={{ id: 'todos-insp', kind: 'todos' }} onNotice={vi.fn()} />);
    expect(await full.findByText('TODO 调度画布')).toBeTruthy(); // full（明细抽屉）模式挂画布
    expect(full.container.querySelector('.oc-todo-run-node[data-todo-id="t1"]')).toBeTruthy();
  });
});
