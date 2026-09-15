// @vitest-environment jsdom
// run.dom.test.jsx — 工作台运行视图 BrainRunView 的 DOM 守卫：
// View = brain_run URL 参数同步 + 返回栏/面包屑 + BrainRunBody，三者共用
// 单层 .brain-run 容器（View 不自裹第二层；旧双层嵌套是 Body/View 拆分
// 残留，flex/gap 逐层等价但 DOM 冗余）。URL 参数只由 View 写——执行明细
// 抽屉的 BrainRunEmbed 直用 Body，不污染浏览器地址。
import '../../../test/setup-dom.js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { BrainRunView } from '../run.jsx';
import { apiGet } from '../../../api.js';

vi.mock('../../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPut: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../../sse.js', () => ({ openStream: vi.fn(() => ({ abort() {} })) }));
// 同 embeds.dom.test.jsx：clearAllMocks 只清调用记录，保住 openStream 替身实现。
afterEach(() => { cleanup(); vi.clearAllMocks(); history.replaceState(null, '', location.pathname); });

describe('工作台运行视图 BrainRunView', () => {
  it('返回栏与运行主体共用单层 .brain-run 容器，View 同步 brain_run 参数并可返回', async () => {
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/brain/runs/run-1') {
        return { objective: '发布里程碑', phase: 'completed', activation: 1, revision: 1, handled_revision: 1, updated_at: 1, total_instances: 0, error: '', input_requests: {}, deliverables: {},
          plan: { id: 'plan-1', version: 2, plan: { steps: [{ id: 's1', label: '第一步', action: { kind: 'agent' } }] } },
          groups: [], instances: [] };
      }
      if (String(path).startsWith('/api/brain/runs/run-1/events-page')) return { events: [], more: false };
      return {};
    });
    const back = vi.fn();
    render(<BrainRunView id="run-1" onBack={back} onNotice={vi.fn()} />);
    expect(await screen.findByText('第一步', { selector: '.brain-step-list strong' })).toBeTruthy(); // Body 主体渲染
    expect(document.querySelectorAll('.brain-run')).toHaveLength(1); // 单层容器：View 不自裹第二层
    expect(new URLSearchParams(window.location.search).get('brain_run')).toBe('run-1'); // URL 参数由 View 同步
    fireEvent.click(screen.getByRole('button', { name: '返回工作台' }));
    expect(back).toHaveBeenCalledOnce();
  });
});
