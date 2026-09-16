// @vitest-environment jsdom
// 导航选择持久化（项目 / Agent / 节点 三分类）DOM 契约：
//   1. 显式导航（Sider Segmented/Menu、移动端 Segmented/Select 都汇入 goPage）
//      把所选页镜像进 localStorage（键 = nav.js NAV_STORAGE_KEY，usehooks-ts
//      JSON 序列化），首次进入不预写；
//   2. 重新挂载 <App/> 时在首帧绘制前恢复（useLayoutEffect → store page 回位、
//      菜单高亮与移动端 Select 同步）；
//   3. 陌生/损坏的存储值（ALL_PAGES 之外）回退默认页；
//   4. brain_run 深链优先于存储值（恢复不落地）。
// 生产代码的 localStorage 读写全部经由 usehooks-ts `useLocalStorage`；测试内
// 用原生 API seeding 属于夹具，不是实现。

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';

// main.jsx mounts <App/> at import time — capture and unmount that stray root
// exactly like app.dom.test.jsx so landmark queries are not doubled.
const { strayRoots } = vi.hoisted(() => ({ strayRoots: [] }));
vi.mock('react-dom/client', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    createRoot: (...args) => {
      const root = actual.createRoot(...args);
      strayRoots.push(root);
      return root;
    },
  };
});

import './test/setup-dom.js';
import App from './main.jsx';
import { NAV_STORAGE_KEY } from './nav.js';
import { clearCredentials, getState, setCredentials, setState } from './store.js';

for (const root of strayRoots.splice(0, strayRoots.length)) {
  root.unmount();
}

// fetch router — restored pages only need their first payload to not throw;
// assertions target navigation state, not panel payloads. Unmatched paths
// resolve to an empty JSON body; /api/project/overview gets the canonical
// empty shape useOverview expects.
const jsonResponse = (body, status = 200) => Promise.resolve({
  ok: status >= 200 && status < 300,
  status,
  json: () => Promise.resolve(body),
});

const installFetchRouter = () => {
  vi.stubGlobal('fetch', vi.fn((input) => {
    const url = typeof input === 'string' ? input : String((input && input.url) || '');
    if (url.includes('/api/me')) {
      return jsonResponse({ name: 'smoke', role: 'admin' });
    }
    if (url.includes('/api/project/overview')) {
      return jsonResponse({ goals: [], backlog: [] });
    }
    if (url.includes('/api/nodes')) {
      return jsonResponse({ nodes: [] });
    }
    return jsonResponse({});
  }));
};

// Console capture: rendering must stay silent of deprecation chatter (same
// gate as app.dom.test.jsx).
const consoleLog = { error: [], warn: [] };
const record = (bucket) => (...args) => {
  consoleLog[bucket].push(args.map((a) => String(a)).join(' '));
};
const deprecationHits = () => consoleLog.error.concat(consoleLog.warn)
  .filter((line) => /deprecated/i.test(line));

beforeEach(() => {
  window.history.replaceState(null, '', '/');
  localStorage.clear();
  clearCredentials();
  setState({ page: 'nodes', preselectNode: null, nodes: [], conn: 'init' });
  installFetchRouter();
  vi.spyOn(console, 'error').mockImplementation(record('error'));
  vi.spyOn(console, 'warn').mockImplementation(record('warn'));
});

afterEach(() => {
  cleanup();
  const hits = deprecationHits();
  consoleLog.error = [];
  consoleLog.warn = [];
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  expect(hits).toEqual([]);
});

describe('导航选择持久化（项目 / Agent / 节点）', () => {
  it('首次进入不预写存储；显式导航把所选页写入 localStorage', async () => {
    setCredentials('smoke-token', '');
    render(<App />);
    expect(localStorage.getItem(NAV_STORAGE_KEY)).toBeNull();

    const sider = within(document.querySelector('.fleet-sidebar'));
    fireEvent.click(sider.getByText('项目')); // 分类 Segmented → 落在首页
    expect(getState().page).toBe('project');
    // usehooks-ts 的 JSON 序列化形状（引号内为页面键）。
    expect(localStorage.getItem(NAV_STORAGE_KEY)).toBe(JSON.stringify('project'));

    fireEvent.click(screen.getByRole('menuitem', { name: /进展/ })); // 分类内换页同样记录
    expect(getState().page).toBe('progress');
    expect(localStorage.getItem(NAV_STORAGE_KEY)).toBe(JSON.stringify('progress'));
  });

  it('重新挂载后恢复上次的分类与页面选择', async () => {
    setCredentials('smoke-token', '');
    localStorage.setItem(NAV_STORAGE_KEY, JSON.stringify('progress'));
    render(<App />);
    // 恢复发生在首帧绘制前：store page 回位，菜单高亮与移动端 Select 同步。
    expect(getState().page).toBe('progress');
    expect(screen.getByRole('menuitem', { name: /进展/ }).classList.contains('ant-menu-item-selected')).toBe(true);
    // antd 6: aria-label 落在内部 input，可见标签在 .ant-select-content。
    const pageNav = screen.getByLabelText('页面导航').closest('.ant-select-content');
    expect(pageNav.textContent).toContain('进展');
  });

  it('陌生或损坏的存储值回退默认页（nodes）', async () => {
    setCredentials('smoke-token', '');
    localStorage.setItem(NAV_STORAGE_KEY, JSON.stringify('fleet')); // 旧构建遗留页
    render(<App />);
    expect(getState().page).toBe('nodes');
    const pageNav = screen.getByLabelText('页面导航').closest('.ant-select-content');
    expect(pageNav.textContent).toContain('节点列表');

    localStorage.setItem(NAV_STORAGE_KEY, '{corrupt');
    render(<App />);
    expect(getState().page).toBe('nodes');
  });

  it('brain_run 深链优先于存储值：恢复不落地', async () => {
    setCredentials('smoke-token', '');
    localStorage.setItem(NAV_STORAGE_KEY, JSON.stringify('project'));
    window.history.replaceState(null, '', '/?brain_run=smoke');
    render(<App />);
    expect(getState().page).toBe('nodes');
  });
});
