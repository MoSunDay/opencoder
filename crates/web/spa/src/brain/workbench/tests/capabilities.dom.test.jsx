// @vitest-environment jsdom
// Capability tab contract: the tab IS the capability CRUD table (BrainPanel) —
// no aggregate /api/brain/library table, no 成熟度 column, no stable/draft
// mutation — and the brain page itself carries no PageShell header.
import '../../../test/setup-dom.js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { BrainWorkbench } from '../index.jsx';
import { apiDel, apiGet, apiPost, apiPut } from '../../../api.js';
vi.mock('../../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPut: vi.fn(), apiDel: vi.fn() }));

const entry = { capability: { id: 'c1', capability_type: 'agent', summary: '解析依赖图', input_desc: 'crate 列表', output_desc: '依赖 DAG', updated_at: 2 }, eng_inputs: [] };
// getAllByRole('button') re-derives an accessible name for every icon-bearing
// antd button (jsdom logs a pseudo-element getComputedStyle error per lookup),
// so toolbar buttons are resolved through their label span instead.
const button = (name) => screen.getByText(name).closest('button');
const openTab = async () => { fireEvent.click(await screen.findByRole('tab', { name: '能力库' })); };

beforeEach(() => {
  vi.resetAllMocks();
  apiGet.mockImplementation(async (path) => {
    if (path === '/api/brain/plan-defs') return { plans: [] };
    if (path === '/api/brain/runs') return { runs: [] };
    if (path === '/api/brain/library') return { capabilities: [] };
    if (path === '/api/brain/capabilities') return { capabilities: [entry] };
    if (path.endsWith('/target')) return { target: null };
    return entry;
  });
  apiPost.mockImplementation(async (path) => (path.endsWith('/search') ? { hits: [] } : { ok: true }));
  apiPut.mockResolvedValue({ ok: true }); apiDel.mockResolvedValue({ ok: true });
});
afterEach(() => { cleanup(); vi.resetAllMocks(); });

describe('brain workbench capability tab', () => {
  it('renders the brain page without a PageShell header', async () => {
    render(<BrainWorkbench onNotice={vi.fn()} />);
    await screen.findByRole('tab', { name: '工作台' });
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
    await waitFor(() => expect(apiGet).toHaveBeenCalledWith('/api/brain/capabilities'));
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
});
