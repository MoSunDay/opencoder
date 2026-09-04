// @vitest-environment jsdom
// TodoPanel DOM smoke: 模板表渲染 fixture（demo / v1），展开行点「运行」命中
// POST /api/todo/templates/:name/:version/run 并跳到「运行」tab；新建模板表单
// 提交命中 POST /api/todo/templates。api.js 模块级 mock（同 queuePanel 模式）；
// sse.js 另以替身 mock —— 它直连 signFetch，而 api.js 的 mock 工厂不含该导出。

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

import './test/setup-dom.js';
import { TodoPanel } from './todoPanel.jsx';

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
    render(<TodoPanel onNotice={() => {}} />);
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
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith('/api/todo/templates/demo/v1/run', {});
    });
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
