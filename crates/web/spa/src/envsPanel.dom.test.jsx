// @vitest-environment jsdom
// EnvsPanel DOM smoke: env 表格渲染 fixture（描述/工具数），行内「编辑」打开
// 抽屉拉取 context（仅描述 + 环境变量；工具走行点击的工具抽屉）；
// 「保存」命中 PUT /api/todo/envs/:name 且 body 只含 description/env_vars
// （tools 由服务端部分合并保留）。工具目录不再全局展示 —— 随 env 进
// envs/toolsDrawer.jsx（其交互见 toolsDrawer.dom.test.jsx）。删除后的
// silent 刷新在途时表格不得遮罩（遮罩 = .ant-spin-spinning → CSS 给容器
// pointer-events: none，行内按钮全锁）。

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

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
import { SPIN_DELAY_MS } from './ui/tableLoading.js';
import { EnvsPanel } from './envsPanel.jsx';

/// jsdom 下只用微任务 flush：RTL 的 waitFor/findBy 依赖真实定时器轮询，
/// 与 fake timers 混用会互相卡住，所以计时相关的用例自己推进时钟。
const flush = async () => { for (let i = 0; i < 6; i += 1) { await act(async () => {}); } };
const advance = async (ms) => { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); };
const isMasked = () => !!document.querySelector('.ant-table-wrapper .ant-spin-spinning');

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

describe('EnvsPanel', () => {
  it('preserves text and variables through rerenders and a failed save', async () => {
    const view = render(<EnvsPanel onNotice={vi.fn()} />);
    await screen.findByText('demo');
    fireEvent.click(findButton('编辑'));
    await waitFor(() => expect(screen.getByLabelText('env-description').value).toBe(demoEnv.description));
    fireEvent.change(screen.getByLabelText('env-description'), { target: { value: 'unsaved description' } });
    fireEvent.change(screen.getByLabelText('var-value'), { target: { value: 'unsaved variable' } });
    const calls = apiGetMock.mock.calls.length;
    view.rerender(<EnvsPanel onNotice={vi.fn()} />);
    await act(async () => {});
    expect(apiGetMock).toHaveBeenCalledTimes(calls);
    let reject;
    apiPutMock.mockImplementation(() => new Promise((_, fail) => { reject = fail; }));
    fireEvent.click(findButton('保存'));
    await waitFor(() => expect(screen.getByLabelText('var-value').disabled).toBe(true));
    expect(screen.getByLabelText('env-description').disabled).toBe(true);
    await act(async () => reject(new Error('save unavailable')));
    expect(screen.getByLabelText('env-description').value).toBe('unsaved description');
    expect(screen.getByLabelText('var-value').value).toBe('unsaved variable');
    expect(screen.getByLabelText('var-value').disabled).toBe(false);
  });

  it('blocks saving when the initial read fails', async () => {
    const delegate = apiGetMock.getMockImplementation();
    apiGetMock.mockImplementation((path) => path === '/api/todo/envs/demo'
      ? Promise.reject(new Error('read unavailable')) : delegate(path));
    const notice = vi.fn();
    render(<EnvsPanel onNotice={notice} />);
    await screen.findByText('demo');
    fireEvent.click(findButton('编辑'));
    await waitFor(() => expect(notice).toHaveBeenCalled());
    expect(findButton('保存').disabled).toBe(true);
    expect(apiPutMock).not.toHaveBeenCalled();
  });

  it('renders the env row; the global tools catalog is gone from the panel', async () => {
    render(<EnvsPanel onNotice={() => {}} />);
    expect(await screen.findByText('demo')).toBeTruthy();
    expect(screen.getByText('视频工具链')).toBeTruthy();
    // 工具数/变量数 Tag（各渲染「N 个」）。
    expect(screen.getAllByText('1 个').length).toBeGreaterThanOrEqual(1);
    // 全局「工具目录」区块已移除：目录随 env 走，进工具抽屉才可见。
    expect(screen.queryByText('工具目录')).toBeNull();
    expect(screen.queryByText('/agent/tools/v2/git')).toBeNull();
  });

  it('selects an env and shows its editor fields', async () => {
    render(<EnvsPanel onNotice={() => {}} />);
    await screen.findByText('demo');
    fireEvent.click(findButton('编辑'));
    expect(await screen.findByText('编辑 Env: demo')).toBeTruthy();
    await waitFor(() => expect(screen.getByLabelText('env-description').value).toBe(demoEnv.description));
    // tools 多选已移出编辑抽屉（绑定改在工具抽屉里管理）。
    expect(screen.queryByLabelText('env-tools')).toBeNull();
    expect(screen.getByLabelText('var-value').value).toBe('/usr/bin/ffmpeg');
  });

  it('opens the tools drawer on row click; 编辑 opens the edit drawer only', async () => {
    render(<EnvsPanel onNotice={() => {}} />);
    await screen.findByText('demo');
    // 操作列 stopPropagation：点「编辑」不触发行点击，工具抽屉不开。
    fireEvent.click(findButton('编辑'));
    expect(await screen.findByText('编辑 Env: demo')).toBeTruthy();
    expect(screen.queryByText('工具: demo')).toBeNull();
    // 点行打开工具抽屉：标题 + 已绑定引用可见。
    fireEvent.click(screen.getByText('demo'));
    expect(await screen.findByText('工具: demo')).toBeTruthy();
    expect(screen.getByText('/agent/tools/v3/ffmpeg')).toBeTruthy();
  });

  it('saves the selected env via PUT (tools preserved server-side)', async () => {
    render(<EnvsPanel onNotice={() => {}} />);
    await screen.findByText('demo');
    fireEvent.click(findButton('编辑'));
    expect(await screen.findByText('编辑 Env: demo')).toBeTruthy();
    await waitFor(() => expect(screen.getByLabelText('env-description').value).toBe(demoEnv.description));
    fireEvent.click(findButton('保存'));
    await waitFor(() => {
      expect(apiPutMock).toHaveBeenCalledWith('/api/todo/envs/demo', {
        description: '视频工具链',
        env_vars: { FFMPEG_PATH: '/usr/bin/ffmpeg' },
      });
    });
  });

  it('keeps the env table unmasked while the post-delete silent refresh is in flight', async () => {
    vi.useFakeTimers();
    try {
      render(<EnvsPanel onNotice={vi.fn()} />);
      await flush();
      expect(screen.getByText('demo')).toBeTruthy();
      expect(isMasked()).toBe(false);

      // 删除成功后的刷新挂住：这一段在途时间就是「行内按钮被锁死」的窗口。
      let releaseRefresh = () => {};
      apiGetMock.mockImplementation((path) => {
        if (path === '/api/todo/envs') {
          return new Promise((resolve) => { releaseRefresh = () => resolve({ envs: [] }); });
        }
        return Promise.resolve(path === '/api/todo/envs/demo' ? { env: demoEnv } : {});
      });
      await act(async () => { fireEvent.click(findButton('删除')); });
      await advance(0);
      await act(async () => { fireEvent.click(findButton('确认删除')); });
      await flush();
      expect(apiDelMock).toHaveBeenCalledWith('/api/todo/envs/demo');

      // 越过 SPIN_DELAY_MS：silent 刷新不该点着 spinner，行数据也不该被抽空。
      await advance(SPIN_DELAY_MS * 2);
      expect(isMasked()).toBe(false);
      expect(document.querySelector('.ant-spin-spinning')).toBeNull();
      expect(screen.getByText('demo')).toBeTruthy();
      expect(findButton('编辑').disabled).toBe(false);

      // 刷新落定后才换数据：此时 loadingEnvs 仍为 false，空态允许显示。
      releaseRefresh();
      await flush();
      expect(screen.getByText('暂无 env')).toBeTruthy();
      expect(isMasked()).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });
});
