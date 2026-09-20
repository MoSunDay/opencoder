// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';

const { apiGetMock, apiPostMock, apiPatchMock, apiDelMock, apiPutMock } = vi.hoisted(() => ({
  apiGetMock: vi.fn(),
  apiPutMock: vi.fn(),
  apiPostMock: vi.fn(),
  apiPatchMock: vi.fn(),
  apiDelMock: vi.fn(),
}));
vi.mock('./api.js', () => ({
  apiGet: apiGetMock,
  apiPut: apiPutMock,
  apiPost: apiPostMock,
  apiPatch: apiPatchMock,
  apiDel: apiDelMock,
}));
vi.mock('./sse.js', () => ({ openStream: vi.fn(() => ({ abort: vi.fn() })) }));

import './test/setup-dom.js';
import { AgentsPanel } from './agentsConfig.jsx';
import { ExecutionDetail } from './fleet/detail.jsx';

/// antd 6 Button 对两字中文自动插空格（「新 建」），按 role + 去空白匹配。
// Filter by accessible name before checking visibility: enumerating every
// button walks the modal's hidden background and is very slow in jsdom.
const findButton = (txt) => screen.getByRole('button', {
  name: (name) => name.replace(/\s+/g, '') === txt,
});

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
    if (/^\/api\/agents\/[^/]+\/meta$/.test(path)) {
      const name = decodeURIComponent(path.split('/')[3]);
      return Promise.resolve({ meta: { name, current: {}, harness: 'opencoder', references: {}, history: [] } });
    }
    if (/\/api\/agents\/[^/]+\/resources\//.test(path)) return Promise.resolve({ baseline: {resource:null,version:0,revision:''}, files:[],versions:[],read_only:false });
    if (path === '/api/harnesses/codex/profiles') return Promise.resolve({ items: [] });
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
  apiPatchMock.mockReset().mockResolvedValue({ ok: true });
  apiDelMock.mockReset().mockResolvedValue({ ok: true, deleted: 'coder' });
};

beforeEach(() => {
  installApi();
});

afterEach(() => {
  cleanup();
});

describe('AgentsPanel', () => {
  it('renders only agent identity in the list and keeps detail-only data out of rows', async () => {
    render(<AgentsPanel onNotice={() => {}} />);
    // 表头筛选下拉与行内 Text 都可能渲染同名文本，用 findAllByText 宽匹配。
    expect((await screen.findAllByText('coder')).length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText('reviewer')).toBeTruthy();
    // 全局激活已删除：列表既没有「生效中」标记，也没有任何全局激活入口。
    expect(screen.queryByText('生效中')).toBeNull();
    expect(screen.queryByLabelText('active-agent')).toBeNull();
    expect(screen.queryByText('base · v2')).toBeNull();
    expect(screen.queryByText('2026-09-01T00:00:00Z')).toBeNull();
    expect(screen.getByRole('tab', { name: 'Agent 列表' })).toBeTruthy();
    expect(screen.getByRole('tab', { name: 'Harness 管理' })).toBeTruthy();
    expect(screen.queryByRole('tab', { name: 'Runner 管理' })).toBeNull();
    expect(screen.queryByRole('tab', { name: 'Agent Harness' })).toBeNull();
    fireEvent.click(screen.getByRole('tab', { name: 'NFS 配置' }));
    expect(await screen.findByText('已停止')).toBeTruthy();
  });

  it('renders registry cards only and offers no builtin scheduling roles', async () => {
    render(<AgentsPanel onNotice={() => {}} />);
    await screen.findAllByText('coder');
    // /api/agents 只返回注册卡：内置调度角色（act/plan/command 等）永不入表。
    for (const builtin of ['act', 'plan', 'explore', 'build', 'sidecar', 'command', 'workflow']) {
      expect(screen.queryByText(builtin)).toBeNull();
    }
  });

  it('filters agents by name through the controlled search box', async () => {
    render(<AgentsPanel onNotice={() => {}} />);
    await screen.findAllByText('coder');
    const search = screen.getByLabelText('agent-search');
    fireEvent.change(search, { target: { value: 'rev' } });
    await waitFor(() => expect(screen.queryByText('coder')).toBeNull());
    expect(screen.getByText('reviewer')).toBeTruthy();
    fireEvent.change(search, { target: { value: 'CODER' } });
    await waitFor(() => expect(screen.getByText('coder')).toBeTruthy());
    expect(screen.queryByText('reviewer')).toBeNull();
    fireEvent.change(search, { target: { value: 'no-such-agent' } });
    expect(await screen.findByText('暂无 agent')).toBeTruthy();
  });

  it('deletes an agent only after the Popconfirm confirm', async () => {
    render(<AgentsPanel onNotice={() => {}} />);
    await screen.findAllByText('coder');
    fireEvent.click(screen.getAllByText(/^删\s*除$/)[0]);
    fireEvent.click(await screen.findByText('确认删除'));
    await waitFor(() => {
      expect(apiDelMock).toHaveBeenCalledWith('/api/agents/coder');
    });
  });

  it('creates an agent by name and harness without resource selects', async () => {
    render(<AgentsPanel onNotice={() => {}} />);
    await screen.findAllByText('coder');
    fireEvent.click(findButton('新建'));
    fireEvent.change(await screen.findByLabelText('new-agent-name'), { target: { value: 'reviewer2' } });
    expect(screen.queryByLabelText('new-agent-prompt')).toBeNull();
    expect(screen.queryByLabelText('new-agent-skills')).toBeNull();
    // 运行模式 Segmented 默认 operator，创建请求始终携带 run_mode。
    const seg = await screen.findByLabelText('new-agent-run-mode');
    expect(seg.closest('.ant-segmented').textContent).toContain('Operator · 宿主机');
    expect(seg.closest('.ant-segmented').textContent).toContain('Agent · runc 沙箱');
    expect(seg.closest('.ant-segmented').querySelector('.ant-segmented-item-selected').textContent)
      .toBe('Operator · 宿主机');
    fireEvent.click(findButton('创建'));
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith('/api/agents', {
        name: 'reviewer2',
        harness: 'opencoder',
        current: {},
        run_mode: 'operator',
      });
    });
    // Modal 两次动效（开/关）在 jsdom 里各吃 ~1.5s，机器高负载下更长（同
    // chat.dom.test 的长测超时惯例，宽放到 20s）。
  }, 20000);

  it('creates an agent with the picked runc sandbox run mode', async () => {
    render(<AgentsPanel onNotice={() => {}} />);
    await screen.findAllByText('coder');
    fireEvent.click(findButton('新建'));
    fireEvent.change(await screen.findByLabelText('new-agent-name'), { target: { value: 'sandboxed' } });
    fireEvent.click(screen.getByText('Agent · runc 沙箱'));
    expect(screen.getByLabelText('new-agent-run-mode').closest('.ant-segmented')
      .querySelector('.ant-segmented-item-selected').textContent).toBe('Agent · runc 沙箱');
    fireEvent.click(findButton('创建'));
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith('/api/agents', {
        name: 'sandboxed',
        harness: 'opencoder',
        current: {},
        run_mode: 'agent',
      });
    });
  }, 20000);

  it('opens agent editing in a right-side 75 percent drawer', async () => {
    render(<AgentsPanel onNotice={() => {}} />);
    await screen.findAllByText('coder');
    fireEvent.click(screen.getAllByText(/^编\s*辑$/)[0]);
    const drawer = await screen.findByRole('dialog');
    expect(within(drawer).getByText('编辑 Agent · coder')).toBeTruthy();
    await within(drawer).findByLabelText('agent-default-harness');
    expect(document.querySelector('.ant-drawer-right')).toBeTruthy();
    expect(screen.getByRole('table', { hidden: true })).toBeTruthy();
    expect(drawer.closest('.ant-drawer')).toBeTruthy();
    expect(drawer.closest('.ant-drawer').querySelector('.ant-drawer-content-wrapper').style.width).toBe('75%');
    fireEvent.click(within(drawer).getByRole('button', { name: 'Close' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    fireEvent.click(screen.getAllByText(/^编\s*辑$/)[1]);
    await within(await screen.findByRole('dialog')).findByText('编辑 Agent · reviewer');
    expect(apiGetMock).toHaveBeenCalledWith('/api/agents/reviewer/meta');
  });

  it('asks before closing a drawer with unsaved resource drafts', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false);
    render(<AgentsPanel onNotice={() => {}} />);
    await screen.findAllByText('coder'); fireEvent.click(screen.getAllByText(/^编\s*辑$/)[0]);
    fireEvent.change(await screen.findByLabelText('prompt-soul'), { target: { value: 'draft' } });
    const drawer = await screen.findByRole('dialog');
    fireEvent.click(within(drawer).getByRole('button', { name: 'Close' }));
    expect(confirm).toHaveBeenCalled(); expect(screen.getByLabelText('prompt-soul').value).toBe('draft');
    confirm.mockReturnValue(true); fireEvent.click(within(drawer).getByRole('button', { name: 'Close' }));
    await waitFor(() => expect(screen.queryByLabelText('prompt-soul')).toBeNull()); confirm.mockRestore();
  });

  it('starts a configured agent and opens its node-owned execution', async () => {
    apiPostMock.mockResolvedValueOnce({ id: 'agent-run-1', kind: 'agent', node_id: 'node-a', created_at: 1, status: 'pending' });
    render(<AgentsPanel onNotice={() => {}} />);
    await screen.findAllByText('coder');
    fireEvent.click(screen.getAllByText(/^启\s*动$/)[0]);
    await pickSelectOption(screen.getByLabelText('agent-harness').closest('.ant-select'), 'Codex');
    expect(screen.queryByLabelText('agent-envs')).toBeNull();
    expect(screen.getByText('Codex 参数已统一管理')).toBeTruthy();
    fireEvent.change(await screen.findByLabelText('任务要求'), { target: { value: '检查发布状态' } });
    fireEvent.click(findButton('启动并查看'));
    await waitFor(() => {
      const call = apiPostMock.mock.calls.find(([path]) => path === '/api/executions');
      expect(call).toBeTruthy();
      expect(call[1]).toMatchObject({ kind: 'agent', target: 'coder', node_id: null, input: { prompt: '检查发布状态', harness: 'codex', envs: {} } });
      expect(call[1].id).toMatch(/^agent-/);
    });
    expect(await screen.findByText('agent-run-1')).toBeTruthy();
  }, 20000);

  it('keeps index identity visible and disables controls while its node is offline', async () => {
    apiGetMock.mockImplementation((path) => {
      if (path === '/api/executions/agent-offline') return Promise.reject(Object.assign(new Error('offline'), { status: 503 }));
      return Promise.resolve({ chunks: [], more: false });
    });
    render(<ExecutionDetail id="agent-offline" summary={{ id: 'agent-offline', kind: 'agent', node_id: 'node-away', created_at: 1, status: 'running' }} onClose={() => {}} onNotice={() => {}} />);
    expect(await screen.findByText('所属节点当前离线，恢复连接后可读取明细和继续操作')).toBeTruthy();
    expect(screen.getByText('node-away')).toBeTruthy();
    expect(findButton('中断（可恢复）').disabled).toBe(true);
    expect(findButton('取消（终止）').disabled).toBe(true);
  }, 10000);
});
