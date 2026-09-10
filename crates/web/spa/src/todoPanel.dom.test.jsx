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
import { TodoRunsPanel, workflowActions } from './todoRunsPanel.jsx';

/// antd 6 Button 对两字中文自动插空格（「创 建」），按 role + 去空白匹配。
const findButton = (txt) => screen.getAllByRole('button')
  .find((b) => (b.textContent || '').replace(/\s+/g, '') === txt);

const templatesFixture = {
  templates: [
    { name: 'demo', description: 'd', current: 'v1', versions: [{ version: 'v1', note: '', created_at: 1 }] },
  ],
};
const detailFixture = { template: templatesFixture.templates[0], env_by_version: { v1: null } };

const installApi = () => {
  apiGetMock.mockReset().mockImplementation((path) => {
    if (path === '/api/todo/templates') {
      return Promise.resolve(templatesFixture);
    }
    if (path === '/api/todo/templates/demo') {
      return Promise.resolve(detailFixture);
    }
    if (path === '/api/todo/workflows?limit=50') {
      return Promise.resolve({ workflows: [{ id: 'todos-1', status: 'running', execution_status: 'running', execution_created_at: 1, node_id: 'node-a', updated_at: 2 }] });
    }
    if (path === '/api/todo/workflows/todos-1') {
      return Promise.resolve({ workflow: { id: 'todos-1', status: 'running' }, items: [] });
    }
    return Promise.resolve({});
  });
  apiPostMock.mockReset().mockResolvedValue({ ok: true, workflow_id: 'todos-1' });
  apiPutMock.mockReset().mockResolvedValue({ ok: true });
  apiDelMock.mockReset().mockResolvedValue({ ok: true });
};

beforeEach(installApi);

afterEach(() => {
  cleanup();
});

describe('TodoPanel 模板 tab', () => {
  it('renders the template table with name and current version', async () => {
    render(<TodoPanel onNotice={() => {}} />);
    expect(await screen.findByText('demo')).toBeTruthy();
    expect(screen.getByText('v1')).toBeTruthy(); // 当前版本列的 Tag
  });

  it('expands a row and dispatches a run for the version', async () => {
    const onNotice = vi.fn();
    apiPostMock.mockRejectedValueOnce(new Error('connection lost')).mockResolvedValueOnce({ ok: true, workflow_id: 'todos-1' });
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
    await waitFor(() => expect(apiPostMock).toHaveBeenCalledTimes(1));
    expect(onNotice).toHaveBeenLastCalledWith(err(expect.stringContaining('connection lost')));
    fireEvent.click(runBtn);
    await waitFor(() => expect(apiPostMock).toHaveBeenCalledTimes(2));
    expect(apiPostMock.mock.calls[0][1].id).toBe(apiPostMock.mock.calls[1][1].id);
    expect(apiPostMock).toHaveBeenLastCalledWith('/api/todo/templates/demo/v1/run', { id: expect.stringMatching(/^todos-/) });
    expect(onNotice).toHaveBeenLastCalledWith(info('已启动工作流: todos-1'));
    // 成功后自动切到「运行」tab（聚焦 todos-1，替身 openStream 不炸即可）。
    await waitFor(() => {
      expect(apiGetMock).toHaveBeenCalledWith('/api/todo/workflows?limit=50');
    });
  });

  it('creates a template through POST /api/todo/templates', async () => {
    render(<TodoPanel onNotice={() => {}} />);
    await screen.findByText('demo');
    fireEvent.click(screen.getByText('新建模板'));
    fireEvent.change(screen.getByLabelText('模板名'), { target: { value: 'spec-check' } });
    fireEvent.click(findButton('创建'));
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith(
        '/api/todo/templates',
        expect.objectContaining({ name: 'spec-check' }),
      );
    });
    const body = apiPostMock.mock.calls.find((c) => c[0] === '/api/todo/templates')[1];
    // 预填的最小示例 spec 原样随请求上行（含 wf-example / t1）。
    expect(body.spec.id).toBe('wf-example');
    expect(body.spec.todos[0].id).toBe('t1');
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
