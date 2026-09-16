// @vitest-environment jsdom
// Embedded execution views retain directory review and report unavailable data.
import '../../test/setup-dom.js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { BrainRunEmbed } from './brainRun.jsx';
import { TodoRunEmbed } from './todoFiles.jsx';
import { ExecutionView } from '../detail.jsx';
import { apiGet } from '../../api.js';

vi.mock('../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPut: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../sse.js', () => ({ openStream: vi.fn(() => ({ abort() {} })) }));

// 不用 vi.resetAllMocks()：它会连 sse.js 替身的实现一起清掉，openStream 返回
// undefined，事件流 effect 的 cleanup 就在 handle.abort() 上炸（同
// todoRunsPanel.dom.test.jsx 的做法）；这里用 clearAllMocks 只清调用记录，
// apiGet 的实现由每个测试自行 mockImplementation 重挂。
Range.prototype.getClientRects=()=>[];
Range.prototype.getBoundingClientRect=()=>({left:0,right:0,top:0,bottom:0});
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

  it('todos 明细默认展示父 Agent 与清单，并保留只读原始记录', async () => {
    apiGet.mockImplementation(async (path) => {
      if (path.includes('section=files')) return {files:{'objective.md':'审核任务'}};
      if (path.startsWith('/api/todo/workflows/todos-1/review?section=node')) return {todo:{title:'实现',agent:'act',acceptance:{criteria:'通过'}},state:{status:'pending',attempt:0}};
      if (path.includes('section=history')) return {events:[],more:false};
      if (path.startsWith('/api/todo/workflows/todos-1/review')) {
        return {workflow:{id:'todos-1',status:'running',generation:1,world_epoch:0},execution_status:'running',head_seq:1,total:2,nodes:[{id:'t1',title:'调研',agent:'plan',depends_on:[],status:'passed',attempt:1},{id:'t2',title:'实现',agent:'act',depends_on:['t1'],status:'pending',attempt:0}]};
      }
      return {};
    });
    render(<TodoRunEmbed id="todos-1" />);
    await waitFor(() => expect(document.querySelector('.todo-task-list [data-todo-id="t1"]')).toBeTruthy());
    expect(screen.getByText('父 Agent 会话尚未就绪')).toBeTruthy();
    expect(document.querySelector('[data-file-path]')).toBeNull();
    fireEvent.click(screen.getByRole('tab',{name:'原始记录'}));
    await waitFor(() => expect(document.querySelector('[data-file-path="process/todos/t1/status.json"]')).toBeTruthy());
    expect(screen.getByText(/1\/2 已通过/)).toBeTruthy();
    fireEvent.click(document.querySelector('[data-file-path="process/todos/t2/status.json"]'));
    await waitFor(() => expect(screen.getByLabelText('文件内容 process/todos/t2/status.json').textContent).toContain('pending'));
    expect(screen.getByLabelText('文件内容 process/todos/t2/status.json').getAttribute('contenteditable')).toBe('false');

  });

  it('todos Review 拉取失败明确报告错误并保留执行信息', async () => {
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/executions/todos-x') {
        return { execution: { id: 'todos-x', kind: 'todos', status: 'done', created_at: 1 },
          request: { kind: 'todos', target: 'tpl/v1', input: {} },
          workflow: { workflow: { status: 'completed' }, items: [{ todo_id: 't1', status: 'done', attempt: 1 }] } };
      }
      if (path.startsWith('/api/todo/workflows/todos-x/review')) throw new Error('node store unreachable'); // 分体部署 node 侧 store 不可见
      return {};
    });
    render(<ExecutionView executionRef={{ id: 'todos-x', kind: 'todos' }} onNotice={vi.fn()} />);
    // 「TODO 工作流」同时是 todos 的类型标签（头部 Descriptions），断言限定
    // 到 TodoDetail 回退块的标题节点。
    expect(await screen.findByText('TODO 工作流', { selector: 'h5' })).toBeTruthy(); // TodoDetail 回退仍在
    expect(await screen.findByText('node store unreachable')).toBeTruthy();
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
      if (path.includes('section=files')) return {files:{'objective.md':'审核任务'}};
      if (path.includes('section=history')) return {events:[],next_before_seq:null};
      if (path.startsWith('/api/todo/workflows/todos-insp/review')) {
        return {workflow:{id:'todos-insp',status:'running',generation:1,world_epoch:0},execution_status:'running',head_seq:1,total:1,nodes:[{id:'t1',title:'内联步骤',agent:'act',depends_on:[],status:'pending',attempt:0}]};
      }
      return {};
    });
    const inline = render(<ExecutionView executionRef={{ id: 'todos-insp', kind: 'todos' }} mode="inline" managed onNotice={vi.fn()} />);
    expect(await inline.findByText('TODO 工作流', { selector: 'h5' })).toBeTruthy(); // TodoDetail 轻量块仍在
    expect(inline.queryByText('TODO 调度画布')).toBeNull(); // 窄列不挂画布
    expect(apiGet.mock.calls.filter(([path]) => path.startsWith('/api/todo/workflows/todos-insp/review'))).toHaveLength(0); // 门控：inline 连画布数据都不拉
    inline.unmount();

    const full = render(<ExecutionView executionRef={{ id: 'todos-insp', kind: 'todos' }} onNotice={vi.fn()} />);
    await waitFor(()=>expect(full.container.querySelector('.todo-task-list [data-todo-id="t1"]')).toBeTruthy());
    expect(full.container.querySelector('.todo-task-list [data-todo-id="t1"]').textContent).toContain('内联步骤');
  });
});
