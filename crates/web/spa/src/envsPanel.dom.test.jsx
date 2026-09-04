// @vitest-environment jsdom
// EnvsPanel DOM smoke: env 列表渲染 fixture，选中后编辑器拉取 context；工具
// 目录的「可导入」行点「导入」命中 POST /api/todo/tools/import；编辑器「保存」
// 命中 PUT /api/todo/envs/:name 且 body 合并 description/tools/env_vars。

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

import './test/setup-dom.js';
import { EnvsPanel } from './envsPanel.jsx';

/// antd 6 Button 对两字中文自动插空格（「导 入」「保 存」），按 role + 去空白匹配。
const findButton = (txt) => screen.getAllByRole('button')
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
    { ref: '/agent/tools/v2/git', source: 'importable', agent: 'agent-1', version: 'v2', tool: 'git' },
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

describe('EnvsPanel', () => {
  it('renders the env row and the tools catalog (share + importable)', async () => {
    render(<EnvsPanel onNotice={() => {}} />);
    expect(await screen.findByText('demo')).toBeTruthy();
    expect(screen.getByText('视频工具链')).toBeTruthy();
    // 可导入表行 + 导入按钮立即可见（未选中 env 也有工具目录）。
    expect(await screen.findByText('/agent/tools/v2/git')).toBeTruthy();
    expect(findButton('导入')).toBeTruthy();
    // 已导入（share）只读清单。
    expect(screen.getByText('已导入（share，只读）：')).toBeTruthy();
  });

  it('selects an env and shows its editor fields', async () => {
    render(<EnvsPanel onNotice={() => {}} />);
    await screen.findByText('demo');
    fireEvent.click(screen.getByText('demo'));
    expect(await screen.findByText('Env: demo')).toBeTruthy();
    // tools 多选框显示已选 share 引用。
    await waitFor(() => {
      expect(screen.getAllByText('/agent/tools/v3/ffmpeg').length).toBeGreaterThanOrEqual(1);
    });
  });

  it('imports an importable tool via POST /api/todo/tools/import', async () => {
    render(<EnvsPanel onNotice={() => {}} />);
    await screen.findByText('/agent/tools/v2/git');
    fireEvent.click(findButton('导入'));
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith('/api/todo/tools/import', {
        agent: 'agent-1',
        version: 'v2',
        tool: 'git',
      });
    });
  });

  it('saves the selected env via PUT with merged body', async () => {
    render(<EnvsPanel onNotice={() => {}} />);
    await screen.findByText('demo');
    fireEvent.click(screen.getByText('demo'));
    expect(await screen.findByText('Env: demo')).toBeTruthy();
    await waitFor(() => {
      expect(screen.getAllByText('/agent/tools/v3/ffmpeg').length).toBeGreaterThanOrEqual(1);
    });
    fireEvent.click(findButton('保存'));
    await waitFor(() => {
      expect(apiPutMock).toHaveBeenCalledWith('/api/todo/envs/demo', {
        description: '视频工具链',
        tools: ['/agent/tools/v3/ffmpeg'],
        env_vars: { FFMPEG_PATH: '/usr/bin/ffmpeg' },
      });
    });
  });
});
