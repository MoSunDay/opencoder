// @vitest-environment jsdom
// BrainPanel DOM smoke: 表单关键控件渲染 + 工程输入可增行；列表渲染两条
// 能力；搜索命中 POST /api/brain/search 且渲染 distance；行删除经
// Popconfirm 确认后命中 DELETE。api.js 模块级 mock（同 todoPanel 模式）。

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';

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

import './test/setup-dom.js';
import { BrainPanel } from './brainPanel.jsx';

/// antd 6 Button 会给中文文案插空格，按 role + 去空白后整体相等匹配。
const findButton = (txt) => screen.getAllByRole('button')
  .find((b) => (b.textContent || '').replace(/\s+/g, '') === txt);

const capsFixture = {
  capabilities: [
    {
      capability: { id: 'c1', capability_type: 'goal', summary: '解析依赖图', input_desc: 'crate 列表', output_desc: '依赖 DAG', created_at: 1, updated_at: 2 },
      eng_inputs: [{ id: 'i1', capability_id: 'c1', content: 'opencoder', position: 0 }],
    },
    {
      capability: { id: 'c2', capability_type: 'constraint', summary: '生成构建计划', input_desc: '目标说明', output_desc: '构建步骤', created_at: 1, updated_at: 2 },
      eng_inputs: [],
    },
  ],
};

beforeEach(() => {
  apiGetMock.mockReset().mockResolvedValue(capsFixture);
  apiPostMock.mockReset().mockImplementation((path, body) => {
    if (path === '/api/brain/search') {
      return Promise.resolve({
        ok: true,
        hits: [{ capability: capsFixture.capabilities[0].capability, distance: 0.123456 }],
      });
    }
    return Promise.resolve({ ok: true });
  });
  apiPutMock.mockReset().mockResolvedValue({ ok: true });
  apiDelMock.mockReset().mockResolvedValue({ ok: true });
});

// 注：setup-dom.js 已在 afterEach 里统一 cleanup()。

describe('BrainPanel', () => {
  it('renders the form landmarks and appends an eng-input row', () => {
    render(<BrainPanel />);
    expect(screen.getByLabelText('能力类型')).toBeTruthy();
    expect(screen.getByLabelText('一句话描述')).toBeTruthy();
    expect(screen.getByLabelText('输入描述')).toBeTruthy();
    expect(screen.getByLabelText('输出描述')).toBeTruthy();
    // Form.List 初始 0 行（后端允许空数组），点「添加工程输入」新增一行。
    expect(screen.queryByPlaceholderText('一条示例输入')).toBeNull();
    fireEvent.click(findButton('+添加工程输入'));
    expect(screen.getByPlaceholderText('一条示例输入')).toBeTruthy();
  });

  it('lists both capabilities from GET /api/brain/capabilities', async () => {
    render(<BrainPanel />);
    expect(await screen.findByText('解析依赖图')).toBeTruthy();
    expect(screen.getByText('生成构建计划')).toBeTruthy();
    expect(screen.getByText('goal')).toBeTruthy(); // 类型 Tag
    expect(screen.getByText('1')).toBeTruthy(); // c1 的工程输入条数
  });

  it('searches via POST /api/brain/search and renders the distance', async () => {
    render(<BrainPanel />);
    await screen.findByText('解析依赖图');
    fireEvent.change(screen.getByPlaceholderText('按意图搜索能力，如：解析依赖图'), { target: { value: '依赖' } });
    fireEvent.click(findButton('搜索'));
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith('/api/brain/search', { query: '依赖', k: 10 });
    });
    expect(await screen.findByText('0.1235')).toBeTruthy(); // 0.123456.toFixed(4)
  });

  it('deletes a capability only after the Popconfirm confirm', async () => {
    render(<BrainPanel />);
    await screen.findByText('解析依赖图');
    fireEvent.click(screen.getAllByText(/^删\s*除$/)[0]);
    fireEvent.click(await screen.findByText('确认删除'));
    await waitFor(() => {
      expect(apiDelMock).toHaveBeenCalledWith('/api/brain/capabilities/c1');
    });
  });

  it('loads a row into the form and saves through PUT', async () => {
    render(<BrainPanel />);
    await screen.findByText('解析依赖图');
    fireEvent.click(screen.getAllByText(/^编\s*辑$/)[0]);
    // 载入后表单带出该条的类型与工程输入行。
    expect(screen.getByDisplayValue('goal')).toBeTruthy();
    expect(screen.getByDisplayValue('opencoder')).toBeTruthy();
    fireEvent.click(findButton('保存修改'));
    await waitFor(() => {
      expect(apiPutMock).toHaveBeenCalledWith(
        '/api/brain/capabilities/c1',
        expect.objectContaining({ capability_type: 'goal', eng_inputs: ['opencoder'] }),
      );
    });
  });
});
