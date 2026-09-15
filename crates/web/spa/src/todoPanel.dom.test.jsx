// @vitest-environment jsdom
// TodoPanel DOM smoke: 模板表渲染 fixture（demo / v1），展开行点「运行」命中
// POST /api/todo/templates/:name/:version/run 并跳到「运行」tab；新建模板表单
// 提交命中 POST /api/todo/templates。api.js 模块级 mock（同 queuePanel 模式）；
// sse.js 另以替身 mock —— 它直连 authFetch，而 api.js 的 mock 工厂不含该导出。

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

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
vi.mock('./fleet/detail.jsx', () => ({ ExecutionDetail: ({ id, summary }) => <div>execution-detail:{id}:{summary?.node_id}</div> }));

import './test/setup-dom.js';
import { err, info } from './notice.js';
import { TodoPanel } from './todoPanel.jsx';
import {specFiles} from './todo/directory/model.js';
import { TodoRunsPanel, workflowActions } from './todoRunsPanel.jsx';

/// antd 6 Button 对两字中文自动插空格（「创 建」），按 role + 去空白匹配。
const findButton = (txt) => [...document.querySelectorAll('button')]
  .find((b) => (b.textContent || '').replace(/\s+/g, '') === txt);

const noopNotice = () => {}; // 稳定引用：TemplatesTab.load 依赖 onNotice，内联箭头会触发无限重取

/// 抽屉契约：右侧滑入、占视口 100%（不叠卡片）。返回 content-wrapper。
const openDrawer = async () => {
  const drawer = await screen.findByRole('dialog');
  expect(drawer.closest('.ant-drawer')).toBeTruthy();
  expect(document.querySelector('.ant-drawer-right')).toBeTruthy();
  const wrapper = drawer.closest('.ant-drawer').querySelector('.ant-drawer-content-wrapper');
  expect(wrapper.style.width).toBe('100%');
  return drawer;
};

const templatesFixture = {
  templates: [
    { name: 'demo', description: 'd', current: 'v1', versions: [{ version: 'v1', note: '', created_at: 1 }] },
  ],
};
const detailFixture = { template: templatesFixture.templates[0], env_by_version: { v1: null } };
/// TodoEditor 拉的三份：context.json（裸 spec）/ envs / env.json。
const SPEC_FIXTURE = {
  schema_version: 1, id: 'wf-demo', name: 'demo', objective: 'ship the demo', constraints: [],
  todos: [{ id: 't1', title: '调研', requirement_background: '背景', instructions: '做', depends_on: [],
    agent: 'act', max_attempts: 3, acceptance: { criteria: '完成' }, metadata: {} }],
  metadata: {},
};

const installApi = () => {
  apiGetMock.mockReset().mockImplementation((path) => {
    if (path === '/api/todo/templates') {
      return Promise.resolve(templatesFixture);
    }
    if (path === '/api/todo/templates/demo') {
      return Promise.resolve(detailFixture);
    }
    if (path === '/api/todo/templates/demo/v1/files') return Promise.resolve({files:specFiles(SPEC_FIXTURE),revision:'r1'});
    if (path === '/api/agents') return Promise.resolve({agents:[{name:'act',primary:true}]});
    if (path === '/api/todo/templates/demo/v1/env.json') {
      return Promise.resolve({ env: null });
    }
    if (path === '/api/todo/envs') {
      return Promise.resolve({ envs: [] });
    }
    if (path === '/api/todo/workflows?limit=50') {
      return Promise.resolve({ workflows: [{ id: 'todos-1', status: 'running', execution_status: 'running', execution_created_at: 1, node_id: 'node-a', updated_at: 2 }] });
    }
    if(path.includes('section=files'))return Promise.resolve({files:specFiles(SPEC_FIXTURE)});
    if(path.includes('section=history'))return Promise.resolve({events:[],next_before_seq:null});
    if (path.startsWith('/api/todo/workflows/todos-1/review')) {
      return Promise.resolve({workflow:{id:'todos-1',status:'running',generation:1,world_epoch:0},execution_status:'running',nodes:[],total:0,head_seq:1,controls:[]});
    }
    return Promise.resolve({});
  });
  apiPostMock.mockReset().mockResolvedValue({ ok: true, workflow_id: 'todos-1' });
  apiPutMock.mockReset().mockResolvedValue({ ok: true });
  apiDelMock.mockReset().mockResolvedValue({ ok: true });
};

beforeEach(()=>{Range.prototype.getClientRects=()=>[];Range.prototype.getBoundingClientRect=()=>({left:0,right:0,top:0,bottom:0});installApi();});

afterEach(() => {
  cleanup();
});

describe('TodoPanel 模板 tab', () => {
  it('renders the template table with name and current version', async () => {
    render(<TodoPanel onNotice={noopNotice} />);
    expect(await screen.findByText('demo')).toBeTruthy();
    expect(screen.getByText('v1')).toBeTruthy(); // 当前版本列的 Tag
  });

  it('expands a row and dispatches a run for the version', async () => {
    const onNotice = vi.fn();
    let attempts=0;
    apiPostMock.mockImplementation(async path=>{if(path.endsWith('/run')&&attempts++===0)throw new Error('connection lost');return {ok:true,workflow_id:'todos-1'};});
    render(<TodoPanel onNotice={onNotice} />);
    await screen.findByText('demo');
    fireEvent.click(document.querySelector('.ant-table-row-expand-icon'));
    expect(await screen.findByText('未绑定 env')).toBeTruthy(); // 版本行 env 徽标
    // 「运行」既是 tab 名也是行按钮：只取 button 载体。
    const runBtn = screen.getAllByText('运行')
      .map((el) => el.closest('button'))
      .filter(Boolean)
      .pop();
    expect(runBtn).toBeTruthy();
    fireEvent.click(runBtn);
    await waitFor(() => expect(apiPostMock.mock.calls.filter(c=>c[0].endsWith('/run'))).toHaveLength(1));
    expect(onNotice).toHaveBeenLastCalledWith(err(expect.stringContaining('connection lost')));
    fireEvent.click(findButton('继续修改'));
    fireEvent.click(runBtn);
    await waitFor(() => expect(apiPostMock.mock.calls.filter(c=>c[0].endsWith('/run'))).toHaveLength(2));
    const runs=apiPostMock.mock.calls.filter(c=>c[0].endsWith('/run'));
    expect(runs[0][1].id).toBe(runs[1][1].id);
    expect(apiPostMock).toHaveBeenLastCalledWith('/api/todo/templates/demo/v1/run', { id: expect.stringMatching(/^todos-/) });
    expect(onNotice).toHaveBeenLastCalledWith(info('已启动工作流: todos-1'));
    // 成功后自动切到「运行」tab（聚焦 todos-1，替身 openStream 不炸即可）。
    await waitFor(() => {
      expect(apiGetMock).toHaveBeenCalledWith('/api/todo/workflows?limit=50');
    });
  });

  it('creates a template through POST /api/todo/templates', async () => {
    render(<TodoPanel onNotice={noopNotice} />);
    await screen.findByText('demo');
    fireEvent.click(screen.getByText('新建模板'));
    await openDrawer();
    fireEvent.change(screen.getByLabelText('模板名'), { target: { value: 'spec-check' } });
    await screen.findByLabelText('文件内容 objective.md');
    fireEvent.click(findButton('创建模板'));
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith(
        '/api/todo/templates',
        expect.objectContaining({ name: 'spec-check' }),
      );
    });
    const body = apiPostMock.mock.calls.find((c) => c[0] === '/api/todo/templates')[1];
    // 预填的最小示例 spec 原样随请求上行（含 wf-example / t1）。
    expect(JSON.parse(body.files['workflow.json']).id).toBe('wf-example');
    expect(body.files['todos/t1/task.json']).toBeTruthy();
  });

  it('opens version editing in a full-width right drawer with the chrome-less editor', async () => {
    render(<TodoPanel onNotice={noopNotice} />);
    await screen.findByText('demo');
    fireEvent.click(document.querySelector('.ant-table-row-expand-icon'));
    fireEvent.click(screen.getByText('编辑'));

    const drawer = await openDrawer();
    expect(screen.getByText('编辑模板 demo')).toBeTruthy(); // 抽屉标题接管 Card 标题
    // 编辑器本体已在抽屉里加载（context 回填 + 三模式切换可用）。
    expect(await screen.findByLabelText('文件内容 objective.md')).toBeTruthy();
    expect(document.querySelector('.file-workspace')).toBeTruthy();
    expect(screen.getByText('workflow.json')).toBeTruthy();
    // 列表仍在抽屉背后（不再整页替换）。
    expect(document.querySelector('.ant-table-row')).toBeTruthy();

    // 返回 = 关抽屉 + 刷新列表（jsdom 不赌 antd 关闭动画，用请求计数验证）。
    // 注意：编辑抽屉展开后 DOM/antd 样式规则剧增，全局 getAllByRole 会因 jsdom
    // getComputedStyle 逐元素解析而慢到分钟级，这里改用原生按钮扫描。
    const listCalls = apiGetMock.mock.calls.filter((c) => c[0] === '/api/todo/templates').length;
    fireEvent.click([...drawer.querySelectorAll('button')]
      .find((b) => (b.textContent || '').replace(/\s+/g, '') === '返回'));
    await waitFor(() => {
      expect(apiGetMock.mock.calls.filter((c) => c[0] === '/api/todo/templates').length).toBe(listCalls + 1);
    });
  });
});

describe('TodoRunsPanel 执行控制', () => {
  it('以节点执行状态决定恢复与终止操作', () => {
    expect(workflowActions('suspended', 'cancelled')).toEqual({ interrupt: false, resume: false, cancel: false });
    expect(workflowActions('failed', 'error')).toEqual({ interrupt: false, resume: true, cancel: false });
    expect(workflowActions('suspended', 'interrupted')).toEqual({ interrupt: false, resume: true, cancel: true });
  });

  it('保留中断、取消与节点执行详情的独立语义', async () => {
    render(<TodoRunsPanel onNotice={vi.fn()} />);
    await screen.findByText(/todos-1/);
    // scroll.x 打开后 tbody 首行是 aria-hidden 的 measure-row，取数据行要带类名。
    fireEvent.click(document.querySelector('tbody tr.ant-table-row'));
    fireEvent.click(await screen.findByText('中断（可恢复）'));
    await waitFor(() => expect(apiPostMock).toHaveBeenCalledWith('/api/todo/workflows/todos-1/interrupt', {}));
    fireEvent.click(screen.getByText('取消（终止）'));
    await waitFor(() => expect(apiPostMock).toHaveBeenCalledWith('/api/executions/todos-1/commands', { action: 'cancel', input: {} }));
    fireEvent.click(screen.getByText('执行详情'));
    expect(await screen.findByText('execution-detail:todos-1:node-a')).toBeTruthy();
  });
});
