// @vitest-environment jsdom
import './test/setup-dom.js';
import { ConfigProvider } from 'antd';
import { beforeEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { BrainPanel } from './brainPanel.jsx';
import { apiDel, apiGet, apiPost, apiPut } from './api.js';
vi.mock('./api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPut: vi.fn(), apiDel: vi.fn() }));

let entries;
const button = (label) => screen.getByText(label, { normalizer: (value) => value.replace(/\s/g, '') }).closest('button');
const visibleRows = (container) => [...container.querySelectorAll('tbody tr[data-row-key]')];
const selectedPage = (container) => container.querySelector('.ant-pagination-item-active').title;
const renderPanel = () => render(<ConfigProvider theme={{ token: { motion: false } }}><BrainPanel executionCatalog={[]} /></ConfigProvider>);
const chooseSize = async (container, size) => {
  fireEvent.mouseDown(container.querySelector('.ant-pagination-options .ant-select'));
  fireEvent.click(await screen.findByText(`${size} / page`, { selector: '.ant-select-item-option-content' }));
};
const next = (container) => fireEvent.click(container.querySelector('.ant-pagination-next'));
const firstId = (container) => visibleRows(container)[0]?.dataset.rowKey;

beforeEach(() => {
  vi.resetAllMocks();
  entries = Array.from({ length: 121 }, (_, index) => ({
    capability: { id: `c${index + 1}`, capability_type: 'legacy-category', summary: `能力 ${index + 1}`, input_desc: '输入', output_desc: '输出', updated_at: 2 },
    eng_inputs: [],
  }));
  apiGet.mockImplementation(async (path) => {
    if (path === '/api/brain/capabilities') return { capabilities: [...entries] };
    if (path === '/api/agents') return { agents: [] };
    if (path.endsWith('/target')) return { target: { kind: 'agent', target: 'act' } };
    return entries.find((entry) => path.endsWith(`/${entry.capability.id}`));
  });
  apiPost.mockResolvedValue({ hits: entries.slice(0, 3).map((entry) => ({ ...entry, distance: 0.1 })) });
  apiPut.mockImplementation(async (path, values) => {
    const entry = entries.find((item) => path.endsWith(`/${item.capability.id}`));
    if (entry) entry.capability = { ...entry.capability, ...values };
    return { ok: true };
  });
  apiDel.mockImplementation(async (path) => {
    entries = entries.filter((entry) => !path.endsWith(`/${entry.capability.id}`));
    return { ok: true };
  });
});

it('shows 100 rows and the remaining 21, and resets to page one for each page size', async () => {
  const { container } = renderPanel();
  await screen.findByText('能力 1');
  expect(visibleRows(container)).toHaveLength(10);
  await chooseSize(container, 100);
  expect(visibleRows(container)).toHaveLength(100);
  expect(firstId(container)).toBe('c1');
  next(container);
  expect(visibleRows(container)).toHaveLength(21);
  expect(firstId(container)).toBe('c101');
  for (const size of [20, 50, 10]) {
    await chooseSize(container, size);
    expect(selectedPage(container)).toBe('1');
    expect(visibleRows(container)).toHaveLength(size);
    expect(firstId(container)).toBe('c1');
    next(container);
    expect(firstId(container)).toBe(`c${size + 1}`);
  }
}, 15000);

it('retains page size and page after refresh and editing, and resets search pages', async () => {
  const { container } = renderPanel();
  await screen.findByText('能力 1');
  await chooseSize(container, 50);
  next(container);
  fireEvent.click(button('刷新'));
  await waitFor(() => expect(apiGet.mock.calls.filter(([path]) => path === '/api/brain/capabilities')).toHaveLength(2));
  expect(firstId(container)).toBe('c51');
  expect(visibleRows(container)).toHaveLength(50);
  fireEvent.click(screen.getByText('能力 51'));
  await waitFor(() => expect(button('保存修改').disabled).toBe(false));
  fireEvent.change(screen.getByLabelText('一句话描述'), { target: { value: '更新能力 51' } });
  fireEvent.click(button('保存修改'));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(await screen.findByText('更新能力 51')).toBeTruthy();
  expect(selectedPage(container)).toBe('2');
  expect(visibleRows(container)).toHaveLength(50);
  fireEvent.change(screen.getByLabelText('搜索能力'), { target: { value: '能力' } });
  fireEvent.click(button('搜索'));
  await screen.findByText('显示全部');
  expect(selectedPage(container)).toBe('1');
  expect(visibleRows(container)).toHaveLength(3);
  fireEvent.click(button('显示全部'));
  expect(selectedPage(container)).toBe('1');
  expect(visibleRows(container)).toHaveLength(50);
  next(container);
  apiPost.mockResolvedValueOnce({ hits: [] });
  fireEvent.change(screen.getByLabelText('搜索能力'), { target: { value: '无匹配' } });
  fireEvent.click(button('搜索'));
  await screen.findByText('没有匹配的能力');
  fireEvent.click(button('显示全部'));
  expect(selectedPage(container)).toBe('1');
  expect(visibleRows(container)).toHaveLength(50);
}, 15000);

it('clamps the last page after deleting its only row and keeps the selected size', async () => {
  const { container } = renderPanel();
  await screen.findByText('能力 1');
  await chooseSize(container, 20);
  fireEvent.click(container.querySelector('.ant-pagination-item[title="7"]'));
  expect(visibleRows(container)).toHaveLength(1);
  fireEvent.click(button('删除'));
  fireEvent.click(await screen.findByText('确认删除'));
  await waitFor(() => expect(firstId(container)).toBe('c101'));
  expect(selectedPage(container)).toBe('6');
  expect(visibleRows(container)).toHaveLength(20);
  expect(screen.queryByText('能力 121')).toBeNull();
  expect(apiDel).toHaveBeenCalledWith('/api/brain/capabilities/c121');
}, 15000);
