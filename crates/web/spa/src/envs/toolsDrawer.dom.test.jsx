// @vitest-environment jsdom
// EnvToolsDrawer DOM smoke：经 EnvsPanel 行点击进入工具抽屉（顺带覆盖面板
// 接线）。打开时并行拉 env 详情 + 工具目录，展示已绑定/可导入与计数；
// 移除/添加走 PUT /api/todo/envs/:name 的部分合并（只发 {tools} 全量列表，
// 成功后 onChanged 静默刷新面板 env 列表）；导入走
// POST /api/todo/tools/import 且只静默重拉目录；PUT 400（工具引用无法解析）
// 经 onNotice 透出且本地绑定保持不变。

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

const { apiGetMock, apiPostMock, apiPutMock, apiDelMock } = vi.hoisted(() => ({
  apiGetMock: vi.fn(),
  apiPostMock: vi.fn(),
  apiPutMock: vi.fn(),
  apiDelMock: vi.fn(),
}));
vi.mock('../api.js', () => ({
  apiGet: apiGetMock,
  apiPost: apiPostMock,
  apiPut: apiPutMock,
  apiDel: apiDelMock,
}));

import '../test/setup-dom.js';
import { EnvsPanel } from '../envsPanel.jsx';

/// jsdom 下只用微任务 flush（同 envsPanel.dom.test.jsx 的约定）。
const flush = async () => { for (let i = 0; i < 6; i += 1) { await act(async () => {}); } };

/// antd 6 Button 对两字中文自动插空格（「移 除」「导 入」「添 加」），按
/// role + 去空白匹配。`hidden: true` 跳过 RTL 的可达性过滤（jsdom 的
/// getComputedStyle × antd cssinjs 样式会让该过滤在抽屉打开后慢两个数量
/// 级），按钮本身均可见，不影响匹配结果。
const findButton = (txt) => screen.getAllByRole('button', { hidden: true })
  .find((b) => (b.textContent || '').replace(/\s+/g, '') === txt);

const demoEnv = {
  name: 'demo',
  description: '视频工具链',
  tools: ['/agent/tools/v3/ffmpeg'],
  env_vars: { FFMPEG_PATH: '/usr/bin/ffmpeg' },
};
const envsFixture = { envs: [demoEnv] };
const toolsFixture = {
  tools: [
    { ref: '/agent/tools/v3/ffmpeg', source: 'share' },
    { ref: '/agent/tools/v1/zip', source: 'share' },
    { ref: '/agent/tools/v2/git', source: 'importable', agent: 'ag', version: 'v2', tool: 'git' },
  ],
};

const installApi = () => {
  apiGetMock.mockReset().mockImplementation((path) => {
    if (path === '/api/todo/envs') {
      return Promise.resolve(envsFixture);
    }
    if (path === '/api/todo/envs/demo') {
      return Promise.resolve({ env: demoEnv });
    }
    if (path === '/api/todo/tools') {
      return Promise.resolve(toolsFixture);
    }
    return Promise.resolve({});
  });
  apiPostMock.mockReset().mockResolvedValue({ ok: true, ref: '/agent/tools/v2/git' });
  apiPutMock.mockReset().mockResolvedValue({ ok: true });
  apiDelMock.mockReset().mockResolvedValue({ ok: true });
};

beforeEach(installApi);

afterEach(() => {
  cleanup();
});

/// 挂载面板并点 demo 行打开工具抽屉，等到绑定行可见；返回 onNotice spy。
const openDrawer = async (notice = vi.fn()) => {
  render(<EnvsPanel onNotice={notice} />);
  fireEvent.click(await screen.findByText('demo'));
  await screen.findByText('工具: demo');
  await screen.findByText('/agent/tools/v3/ffmpeg');
  await flush();
  return notice;
};

describe('EnvToolsDrawer', () => {
  it('shows bound tools, the importable catalog and counts on row click', async () => {
    await openDrawer();
    // 抽屉头部展示描述，同时面板表格行仍保留同一描述文案。
    expect(screen.getAllByText('视频工具链').length).toBeGreaterThan(0);
    expect(screen.getByText('工具 1 个')).toBeTruthy();
    expect(screen.getByText('变量 1 个')).toBeTruthy();
    expect(screen.getByText('/agent/tools/v2/git')).toBeTruthy(); // 可导入表行
    expect(findButton('移除')).toBeTruthy();
    expect(findButton('导入')).toBeTruthy();
  });

  it('removes a bound tool via PUT with the emptied tools list', async () => {
    await openDrawer();
    fireEvent.click(findButton('移除'));
    await waitFor(() => {
      expect(apiPutMock).toHaveBeenCalledWith('/api/todo/envs/demo', { tools: [] });
    });
    await waitFor(() => expect(screen.getByText('未绑定工具')).toBeTruthy());
    // onChanged 让面板静默刷新 env 列表。
    await waitFor(() => {
      expect(apiGetMock.mock.calls.filter(([p]) => p === '/api/todo/envs').length).toBe(2);
    });
  });

  it('adds a pending share tool via PUT with the deduped full tools list', async () => {
    await openDrawer();
    fireEvent.mouseDown(screen.getByLabelText('env-add-tools').closest('.ant-select'));
    fireEvent.click(
      await screen.findByText('/agent/tools/v1/zip', { selector: '.ant-select-item-option-content' }),
    );
    fireEvent.click(findButton('添加'));
    await waitFor(() => {
      expect(apiPutMock).toHaveBeenCalledWith('/api/todo/envs/demo', {
        tools: ['/agent/tools/v3/ffmpeg', '/agent/tools/v1/zip'],
      });
    });
  });

  it('imports an importable tool via POST and silently refreshes the catalog', async () => {
    await openDrawer();
    fireEvent.click(findButton('导入'));
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith('/api/todo/tools/import', {
        agent: 'ag',
        version: 'v2',
        tool: 'git',
      });
    });
    // 导入只改目录不改绑定：静默重拉 /api/todo/tools，不动 env 列表。
    await waitFor(() => {
      expect(apiGetMock.mock.calls.filter(([p]) => p === '/api/todo/tools').length).toBe(2);
    });
    expect(apiGetMock.mock.calls.filter(([p]) => p === '/api/todo/envs').length).toBe(1);
    expect(apiPutMock).not.toHaveBeenCalled();
  });

  it('surfaces a PUT failure via onNotice and keeps the binding', async () => {
    apiPutMock.mockRejectedValue(new Error('工具引用无法解析: /agent/tools/v3/ffmpeg: not found'));
    const notice = await openDrawer();
    fireEvent.click(findButton('移除'));
    await waitFor(() => expect(notice).toHaveBeenCalled());
    const note = notice.mock.calls.map((c) => c[0]).find((n) => n && n.text);
    expect(note.type).toBe('error');
    expect(note.text).toContain('更新工具失败');
    expect(note.text).toContain('工具引用无法解析');
    // 本地绑定不变：失败不吞掉已绑定行。
    expect(screen.getByText('/agent/tools/v3/ffmpeg')).toBeTruthy();
  });
});
