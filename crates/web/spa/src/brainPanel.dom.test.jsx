// @vitest-environment jsdom
import './test/setup-dom.js';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { BrainPanel } from './brainPanel.jsx';
import { apiDel, apiGet, apiPost, apiPut } from './api.js';
vi.mock('./api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPut: vi.fn(), apiDel: vi.fn() }));

const entry = { capability: { id: 'c1', capability_type: 'goal', summary: '解析依赖图', input_desc: 'crate 列表', output_desc: '依赖 DAG', updated_at: 2 }, eng_inputs: [{ content: 'opencoder' }] };
const second = { capability: { ...entry.capability, id: 'c2', summary: '生成构建计划' }, eng_inputs: [] };
const button = (name) => screen.getAllByRole('button').find((item) => item.textContent.replace(/\s/g, '') === name);
const fill = () => {
  for (const [label, value] of [['能力类型', 'goal'], ['一句话描述', '新能力'], ['输入描述', '需求'], ['输出描述', '结果']]) {
    fireEvent.change(screen.getByLabelText(label), { target: { value } });
  }
};

beforeEach(() => {
  vi.resetAllMocks();
  apiGet.mockImplementation(async (path) => {
    if (path === '/api/brain/capabilities') return { capabilities: [entry, second] };
    if (path.endsWith('/target')) return { target: null };
    return entry;
  });
  apiPost.mockImplementation(async (path) => path.endsWith('/search')
    ? { hits: [{ capability: entry.capability, distance: 0.123456 }] }
    : { capability: { id: 'created' } });
  apiPut.mockResolvedValue({ ok: true }); apiDel.mockResolvedValue({ ok: true });
});

describe('capability library table and editor', () => {
  it('shows a single table and keeps all forms in the drawer', async () => {
    const { container } = render(<BrainPanel />);
    expect(await screen.findByText('解析依赖图')).toBeTruthy();
    expect(screen.getByText('生成构建计划')).toBeTruthy();
    expect(container.querySelectorAll('.ant-table')).toHaveLength(1);
    expect(container.querySelector('.ant-card')).toBeNull();
    expect(screen.queryByLabelText('能力类型')).toBeNull();
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('creates through a 75 percent right drawer and resets fields for the next creation', async () => {
    render(<BrainPanel />);
    fireEvent.click(button('新建能力'));
    const drawer = screen.getByRole('dialog');
    expect(drawer.closest('.ant-drawer').className).toContain('ant-drawer-right');
    expect(drawer.closest('.ant-drawer-content-wrapper').style.width).toBe('75%');
    fill(); fireEvent.click(button('添加工程输入'));
    fireEvent.change(screen.getByPlaceholderText('一条示例输入'), { target: { value: '示例' } });
    fireEvent.click(button('创建能力'));
    await waitFor(() => expect(apiPost).toHaveBeenCalledWith('/api/brain/capabilities', {
      capability_type: 'goal', summary: '新能力', input_desc: '需求', output_desc: '结果', eng_inputs: ['示例'],
    }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(apiPut).not.toHaveBeenCalled();
    fireEvent.click(button('新建能力'));
    expect(screen.getByLabelText('一句话描述').value).toBe('');
    expect(screen.queryByPlaceholderText('一条示例输入')).toBeNull();
  });

  it('opens a clicked table row in edit mode and saves the full content through PUT', async () => {
    render(<BrainPanel />);
    fireEvent.click(await screen.findByText('解析依赖图'));
    expect(await screen.findByText('编辑能力', { selector: '.ant-drawer-title' })).toBeTruthy();
    await waitFor(() => expect(screen.getByDisplayValue('opencoder').disabled).toBe(false));
    fireEvent.change(screen.getByLabelText('一句话描述'), { target: { value: '更新后的能力' } });
    fireEvent.click(button('保存修改'));
    await waitFor(() => expect(apiPut).toHaveBeenCalledWith('/api/brain/capabilities/c1', expect.objectContaining({ summary: '更新后的能力', eng_inputs: ['opencoder'] })));
    expect(apiPost).not.toHaveBeenCalled();
  });

  it('renders search hits in the same table and fetches exemplars before editing', async () => {
    const { container } = render(<BrainPanel />);
    await screen.findByText('解析依赖图');
    fireEvent.change(screen.getByLabelText('搜索能力'), { target: { value: '依赖' } });
    fireEvent.click(button('搜索'));
    expect(await screen.findByText('0.1235')).toBeTruthy();
    expect(apiPost).toHaveBeenCalledWith('/api/brain/search', { query: '依赖', k: 10 });
    expect(container.querySelectorAll('.ant-table')).toHaveLength(1);
    expect(screen.queryByText('生成构建计划')).toBeNull();
    fireEvent.click(screen.getByText('解析依赖图'));
    expect(await screen.findByDisplayValue('opencoder')).toBeTruthy();
    expect(apiGet).toHaveBeenCalledWith('/api/brain/capabilities/c1');
    fireEvent.click(button('取消'));
    fireEvent.click(button('显示全部'));
    expect(await screen.findByText('生成构建计划')).toBeTruthy();
  });

  it('distinguishes a cancelled edit from a blank new capability', async () => {
    render(<BrainPanel />);
    fireEvent.click(await screen.findByText('解析依赖图'));
    await screen.findByDisplayValue('opencoder');
    fireEvent.click(button('取消'));
    fireEvent.click(button('新建能力'));
    expect(screen.getByText('新建能力', { selector: '.ant-drawer-title' })).toBeTruthy();
    expect(screen.getByLabelText('一句话描述').value).toBe('');
    expect(screen.queryByDisplayValue('opencoder')).toBeNull();
    expect(apiPost).not.toHaveBeenCalled(); expect(apiPut).not.toHaveBeenCalled();
  });

  it('does not open an editor from the delete action and waits for confirmation', async () => {
    render(<BrainPanel />);
    const row = (await screen.findByText('解析依赖图')).closest('tr');
    fireEvent.click(within(row).getByRole('button', { name: /删\s*除/ }));
    expect(apiDel).not.toHaveBeenCalled(); expect(screen.queryByRole('dialog')).toBeNull();
    fireEvent.click(await screen.findByText('确认删除'));
    await waitFor(() => expect(apiDel).toHaveBeenCalledWith('/api/brain/capabilities/c1'));
  });

  it('retains content and reuses the created ID when saving its target fails', async () => {
    render(<BrainPanel />); fireEvent.click(button('新建能力')); fill();
    fireEvent.change(screen.getByLabelText('执行目标'), { target: { value: 'custom-agent' } });
    apiPut.mockRejectedValueOnce(new Error('connection lost'));
    fireEvent.click(button('创建能力'));
    expect(await screen.findByText(/能力内容已保存，执行目标未保存/)).toBeTruthy();
    expect(screen.getByLabelText('一句话描述').value).toBe('新能力');
    fireEvent.click(button('保存修改'));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(apiPost).toHaveBeenCalledTimes(1);
    expect(apiPut).toHaveBeenCalledWith('/api/brain/capabilities/created', expect.objectContaining({ summary: '新能力' }));
    expect(apiPut).toHaveBeenLastCalledWith('/api/brain/capabilities/created/target', { kind: 'agent', target: 'custom-agent' });
  });

  it('blocks saving if the latest capability cannot be loaded', async () => {
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/brain/capabilities') return { capabilities: [entry] };
      throw new Error('node unavailable');
    });
    render(<BrainPanel />); fireEvent.click(await screen.findByText('解析依赖图'));
    expect(await screen.findByText('读取能力失败: node unavailable')).toBeTruthy();
    expect(button('保存修改').disabled).toBe(true);
    expect(apiPut).not.toHaveBeenCalled();
  });
});
