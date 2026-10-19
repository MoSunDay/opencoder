// @vitest-environment jsdom
// Capability tab contract: the tab IS the capability CRUD table (BrainPanel) —
// no aggregate /api/brain/library table, no 成熟度 column, no stable/draft
// mutation — and the brain page itself carries no PageShell header.
import '../../../test/setup-dom.js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { BrainWorkbench } from '../index.jsx';
import { apiDel, apiGet, apiPost, apiPut } from '../../../api.js';
vi.mock('../../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPut: vi.fn(), apiDel: vi.fn() }));

const entry = { capability: { id: 'c1', capability_type: 'agent', summary: '解析依赖图', input_desc: 'crate 列表', output_desc: '依赖 DAG', updated_at: 2 }, eng_inputs: [] };
let bindings;
let entries;
// getAllByRole('button') re-derives an accessible name for every icon-bearing
// antd button (jsdom logs a pseudo-element getComputedStyle error per lookup),
// so toolbar buttons are resolved through their label span instead.
const button = (name) => screen.getByText(name, { normalizer: (value) => value.replace(/\s/g, '') }).closest('button');
const openTab = async () => { fireEvent.click(await screen.findByRole('tab', { name: '能力库' })); };

beforeEach(() => {
  vi.resetAllMocks();
  bindings = [{ id: 'c1', kind: 'operator', target: 'generic-transform', unavailable_reason: '定义不存在' }, { id: 'auto-only', kind: 'agent', target: 'automatic-agent' }];
  entries = [entry];
  apiGet.mockImplementation(async (path) => {
    if (path === '/api/brain/plan-defs') return { plans: [] };
    if (path.startsWith('/api/brain/runs')) return { runs: [] };
    if (path === '/api/brain/library') return { capabilities: [...bindings] };
    if (path === '/api/brain/capabilities') return { capabilities: [...entries] };
    if (path === '/api/agents') return { agents: [{ name: 'new-agent' }] };
    if (path.endsWith('/target')) return { target: bindings.find((item) => path.includes(`/${item.id}/`)) || null };
    return entry;
  });
  apiPost.mockImplementation(async (path) => (path.endsWith('/search') ? { hits: [] } : { ok: true }));
  apiPut.mockResolvedValue({ ok: true }); apiDel.mockResolvedValue({ ok: true });
});
afterEach(() => { cleanup(); vi.resetAllMocks(); });

describe('brain workbench capability tab', () => {
  it('renders the brain page without a PageShell header', async () => {
    render(<BrainWorkbench onNotice={vi.fn()} />);
    await screen.findByRole('tab', { name: '运行历史' });
    expect(screen.queryByRole('heading', { name: '大脑调度' })).toBeNull();
    expect(screen.queryByText('维护能力与版本化计划，观察并发执行和交付证据')).toBeNull();
    expect(screen.queryByText('创建并执行计划')).toBeNull();
  });

  it('shows the capability CRUD table directly with no maturity rows or library mutations', async () => {
    const { container } = render(<BrainWorkbench onNotice={vi.fn()} />);
    await openTab();
    expect(await screen.findByText('解析依赖图')).toBeTruthy();
    // The Collapse wrapper around BrainPanel is gone…
    expect(screen.queryByText('维护能力描述与目标绑定')).toBeNull();
    // …and so is the aggregate library table (maturity + expandable rows).
    expect(screen.queryByText('成熟度')).toBeNull();
    expect(screen.queryByText('标记稳定')).toBeNull();
    expect(screen.queryByText('改为草稿')).toBeNull();
    expect(container.querySelector('.ant-table-row-expand-icon')).toBeNull();
    expect(button('新建能力')).toBeTruthy();
    await waitFor(() => expect(apiGet).toHaveBeenCalledWith('/api/brain/capabilities', expect.objectContaining({ signal: expect.any(AbortSignal) })));
    expect(apiPost.mock.calls.filter(([path]) => String(path).includes('/api/brain/library/'))).toEqual([]);
  });

  it('opens the 新建能力 dialog from the capability tab', async () => {
    render(<BrainWorkbench onNotice={vi.fn()} />);
    await openTab();
    await screen.findByText('解析依赖图');
    fireEvent.click(button('新建能力'));
    expect(screen.getByRole('dialog')).toBeTruthy();
    expect(screen.getByText('新建能力', { selector: '.ant-drawer-title' })).toBeTruthy();
  });

  it('opens the 编辑能力 drawer when a capability row is clicked', async () => {
    render(<BrainWorkbench onNotice={vi.fn()} />);
    await openTab();
    fireEvent.click(await screen.findByText('解析依赖图'));
    expect(await screen.findByText('编辑能力', { selector: '.ant-drawer-title' })).toBeTruthy();
  });

  it('joins actual bindings by ID for list and search without adding aggregate-only rows', async () => {
    entries.push({ capability: { ...entry.capability, id: 'unbound', summary: '未配置的能力' } });
    apiPost.mockResolvedValue({ hits: [{ capability: entry.capability, distance: 0.25 }] });
    const { container } = render(<BrainWorkbench />);
    await openTab();
    expect(await screen.findByText('generic-transform')).toBeTruthy();
    expect(screen.getByText('Operator')).toBeTruthy();
    expect(screen.queryByText('Agent')).toBeNull();
    expect(screen.queryByText('automatic-agent')).toBeNull();
    expect(screen.getAllByText('未绑定')).toHaveLength(2);
    expect(container.querySelectorAll('.brain-capability-table tr[data-row-key]')).toHaveLength(2);
    // The list reuses the shared catalog; individual bindings are only read by the editor.
    expect(apiGet.mock.calls.filter(([path]) => path.endsWith('/target'))).toHaveLength(0);
    expect(apiGet.mock.calls.filter(([path]) => path === '/api/brain/library')).toHaveLength(2);
    fireEvent.change(screen.getByLabelText('搜索能力'), { target: { value: '依赖' } });
    fireEvent.click(button('搜索'));
    await screen.findByText('0.2500');
    expect(screen.getByText('generic-transform')).toBeTruthy();
    expect(screen.getByText('Operator')).toBeTruthy();
    expect(screen.queryByText('未绑定')).toBeNull();
  });

  it('refreshes bindings after refresh, saving a new target and deleting a capability', async () => {
    render(<BrainWorkbench />);
    await openTab();
    await screen.findByText('generic-transform');
    bindings = [{ id: 'c1', kind: 'agent', target: 'old-agent' }];
    fireEvent.click(button('刷新'));
    expect(await screen.findByText('old-agent')).toBeTruthy();
    expect(screen.queryByText('generic-transform')).toBeNull();
    fireEvent.click(screen.getByText('解析依赖图'));
    await waitFor(() => expect(button('保存修改').disabled).toBe(false));
    fireEvent.mouseDown(screen.getByLabelText('名称').closest('.ant-select'));
    fireEvent.click(await screen.findByText('new-agent', { selector: '.ant-select-item-option-content' }));
    apiPut.mockImplementation(async (path, body) => {
      if (path.endsWith('/target')) bindings = [{ id: 'c1', ...body }];
      return { ok: true };
    });
    fireEvent.click(button('保存修改'));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(await screen.findByText('new-agent', { selector: '.brain-capability-name' })).toBeTruthy();
    expect(screen.queryByText('old-agent')).toBeNull();
    apiDel.mockImplementation(async () => { entries = []; bindings = []; return { ok: true }; });
    const reads = apiGet.mock.calls.filter(([path]) => path === '/api/brain/library').length;
    fireEvent.click(button('删除'));
    fireEvent.click(await screen.findByText('确认删除'));
    await screen.findByText('暂无能力，点击「新建能力」添加');
    expect(screen.queryByText('new-agent')).toBeNull();
    expect(apiGet.mock.calls.filter(([path]) => path === '/api/brain/library')).toHaveLength(reads + 1);
  });

  it('shows a binding read failure instead of unbound and retries independently of other lists', async () => {
    const get = apiGet.getMockImplementation();
    apiGet.mockImplementation(async (path, options) => {
      if (path === '/api/brain/library') throw new Error('catalog unavailable');
      if (path === '/api/brain/plan-defs') throw new Error('plans unavailable');
      return get(path, options);
    });
    render(<BrainWorkbench />);
    await openTab();
    await screen.findByText('解析依赖图');
    expect(await screen.findByText('获取执行绑定失败: catalog unavailable')).toBeTruthy();
    expect(screen.getAllByText('加载失败')).toHaveLength(2);
    expect(screen.queryByText('未绑定')).toBeNull();
    apiGet.mockImplementation(get);
    fireEvent.click(screen.getByText('获取执行绑定失败: catalog unavailable').closest('.ant-alert').querySelector('button'));
    expect(await screen.findByText('generic-transform')).toBeTruthy();
    expect(screen.queryByText('加载失败')).toBeNull();
    expect(screen.getByText('plans unavailable')).toBeTruthy();
  });

  it('ignores superseded catalog reads and aborts pending reads on unmount', async () => {
    const get = apiGet.getMockImplementation();
    const requests = [];
    apiGet.mockImplementation((path, options) => path === '/api/brain/library'
      ? new Promise((resolve) => requests.push({ resolve, signal: options.signal })) : get(path, options));
    const { unmount } = render(<BrainWorkbench />);
    await waitFor(() => expect(requests).toHaveLength(1));
    await openTab();
    await screen.findByText('解析依赖图');
    expect(screen.getAllByText('加载中')).toHaveLength(2);
    expect(screen.queryByText('未绑定')).toBeNull();
    expect(requests).toHaveLength(2);
    expect(requests[0].signal.aborted).toBe(true);
    await act(async () => requests[1].resolve({ capabilities: [{ id: 'c1', kind: 'dag', target: 'current-workflow' }] }));
    await screen.findByText('current-workflow');
    await act(async () => requests[0].resolve({ capabilities: bindings }));
    expect(screen.queryByText('generic-transform')).toBeNull();
    expect(screen.getByText('DAG')).toBeTruthy();
    fireEvent.click(button('刷新'));
    expect(requests).toHaveLength(3);
    unmount();
    expect(requests[2].signal.aborted).toBe(true);
    const listReads = apiGet.mock.calls.filter(([path]) => path === '/api/brain/capabilities');
    expect(listReads.at(-1)[1].signal.aborted).toBe(true);
    await act(async () => requests[2].resolve({ capabilities: [] }));
  });
});
