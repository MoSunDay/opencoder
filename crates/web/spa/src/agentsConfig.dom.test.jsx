// @vitest-environment jsdom
// AgentsPanel DOM smoke：agent 表渲染 fixture（引用 tag / 生效徽标 / `—`），
// 生效 Select 换选命中 PATCH /api/agents/active 且 body 带 active，删除走
// Popconfirm 确认后命中 DELETE /api/agents/:name，新建 modal 提交命中
// POST /api/agents（未选引用 ⇒ null）。api.js 模块级 mock（同 envsPanel 模式）。

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

const { apiGetMock, apiPostMock, apiPatchMock, apiDelMock } = vi.hoisted(() => ({
  apiGetMock: vi.fn(),
  apiPostMock: vi.fn(),
  apiPatchMock: vi.fn(),
  apiDelMock: vi.fn(),
}));
vi.mock('./api.js', () => ({
  apiGet: apiGetMock,
  apiPost: apiPostMock,
  apiPatch: apiPatchMock,
  apiDel: apiDelMock,
}));

import './test/setup-dom.js';
import { AgentsPanel } from './agentsConfig.jsx';

/// antd 6 Button 对两字中文自动插空格（「新 建」），按 role + 去空白匹配。
const findButton = (txt) => screen.getAllByRole('button')
  .find((b) => (b.textContent || '').replace(/\s+/g, '') === txt);

/// 打开指定 antd 6 Select（交互面是 .ant-select 根）并在浮层里点 `label`。
/// options portal 到 document.body，凭 .ant-select-item-option 的 title 匹配。
const pickSelectOption = async (selectEl, label) => {
  await act(async () => {
    fireEvent.mouseDown(selectEl);
  });
  const option = await waitFor(() => {
    const all = [...document.querySelectorAll('.ant-select-item-option')];
    const hit = all.find((o) => o.getAttribute('title') === label || o.textContent === label);
    expect(hit).toBeTruthy();
    return hit;
  });
  await act(async () => {
    fireEvent.click(option);
  });
};

const agentsFixture = {
  ok: true,
  active: 'coder',
  agents: [
    {
      name: 'coder',
      current: { prompt: 'base', skills: null, tools: 'std', memory: null },
      references: { prompt_files: ['soul'], skills: [], tools: ['bash'], memory: false },
      updated_at: '2026-09-01T00:00:00Z',
    },
    {
      name: 'reviewer',
      current: { prompt: null, skills: null, tools: null, memory: null },
      references: { prompt_files: [], skills: [], tools: [], memory: false },
      updated_at: '',
    },
  ],
};
const promptsFixture = {
  ok: true,
  category: 'prompts',
  resources: [
    { name: 'base', current: 2, versions: [1, 2] },
    { name: 'alt', current: 1, versions: [1] },
  ],
};

const installApi = () => {
  apiGetMock.mockReset().mockImplementation((path) => {
    if (path === '/api/agents') {
      return Promise.resolve(agentsFixture);
    }
    if (path === '/api/agents/resources/prompts') {
      return Promise.resolve(promptsFixture);
    }
    if (path === '/api/agents/nfs') {
      return Promise.resolve({
        ok: true,
        status: { running: false, host: '', port: 0, read_only: true, export_root: '' },
      });
    }
    return Promise.resolve({ ok: true, resources: [] });
  });
  apiPostMock.mockReset().mockResolvedValue({ ok: true, name: 'x' });
  apiPatchMock.mockReset().mockResolvedValue({ ok: true, active: 'reviewer' });
  apiDelMock.mockReset().mockResolvedValue({ ok: true, deleted: 'coder' });
};

beforeEach(() => {
  installApi();
});

afterEach(() => {
  cleanup();
});

describe('AgentsPanel', () => {
  it('renders agent rows with ref tags, active badge and updated_at', async () => {
    render(<AgentsPanel onNotice={() => {}} />);
    // 生效 Select 的选中项与表格行同名 —— 用 findAllByText 断言两处都在。
    expect((await screen.findAllByText('coder')).length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText('reviewer')).toBeTruthy();
    // 生效徽标只在 coder 行。
    expect(screen.getByText('生效中')).toBeTruthy();
    // 引用 tag：已引用带值，未引用显示 `—`。
    expect(screen.getByText('Prompt: base')).toBeTruthy();
    expect(screen.getByText('Tools: std')).toBeTruthy();
    expect(screen.getAllByText('Skills: —').length).toBe(2);
    expect(screen.getAllByText('Memory: —').length).toBe(2);
    expect(screen.getByText('2026-09-01T00:00:00Z')).toBeTruthy();
    // NFS 卡片随页渲染（已停止态）。
    expect(await screen.findByText('已停止')).toBeTruthy();
  });

  it('fires PATCH /api/agents/active when the active select changes', async () => {
    const { container } = render(<AgentsPanel onNotice={() => {}} />);
    await screen.findByText('Prompt: base');
    await pickSelectOption(container.querySelector('.ant-select'), 'reviewer');
    await waitFor(() => {
      expect(apiPatchMock).toHaveBeenCalledWith('/api/agents/active', { active: 'reviewer' });
    });
  });

  it('deletes an agent only after the Popconfirm confirm', async () => {
    render(<AgentsPanel onNotice={() => {}} />);
    await screen.findByText('Prompt: base');
    fireEvent.click(screen.getAllByText(/^删\s*除$/)[0]);
    fireEvent.click(await screen.findByText('确认删除'));
    await waitFor(() => {
      expect(apiDelMock).toHaveBeenCalledWith('/api/agents/coder');
    });
  });

  it('creates an agent through POST with null refs for untouched selects', async () => {
    render(<AgentsPanel onNotice={() => {}} />);
    await screen.findByText('Prompt: base');
    fireEvent.click(findButton('新建'));
    fireEvent.change(await screen.findByLabelText('new-agent-name'), { target: { value: 'reviewer2' } });
    // modal 的 prompt Select 是文档里第二个 .ant-select（首个是生效选择）。
    const selects = document.querySelectorAll('.ant-select');
    await pickSelectOption(selects[1], 'base · v2');
    fireEvent.click(findButton('创建'));
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith('/api/agents', {
        name: 'reviewer2',
        current: { prompt: 'base', skills: null, tools: null, memory: null },
      });
    });
    // Modal 两次动效（开/关）在 jsdom 里各吃 ~1.5s，机器高负载下更长（同
    // chat.dom.test 的长测超时惯例，宽放到 20s）。
  }, 20000);
});
