// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { apiGet } from '../../../api.js';
import { useHistory } from './useHistory.js';
import { RunHistory } from './index.jsx';
vi.mock('../../../api.js', () => ({ apiGet: vi.fn() }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });
function Harness() {
  const history = useHistory();
  return <RunHistory history={history} onOpen={vi.fn()} />;
}
const row = (id, name) => ({ id, name, status: 'done', created_at: 1, node_id: 'node-a' });
it('按冻结名称和执行 ID 展示每次执行，游标分页及刷新保持当前页', async () => {
  apiGet.mockResolvedValueOnce({ runs: [row('brain-a', '同一计划'), row('brain-b', '同一计划')], next_cursor: { id: 'brain-b', created_at: 1 } })
    .mockResolvedValueOnce({ runs: [row('brain-old')], next_cursor: null })
    .mockResolvedValueOnce({ runs: [row('brain-old')], next_cursor: null })
    .mockResolvedValueOnce({ runs: [row('brain-a', '同一计划')], next_cursor: null });
  render(<Harness />);
  expect(await screen.findAllByRole('button', { name: '同一计划' })).toHaveLength(2);
  fireEvent.click(screen.getByRole('button', { name: '下一页' }));
  expect(await screen.findByText('brain-old')).toBeTruthy();
  expect(screen.getByText('未记录计划名称')).toBeTruthy();
  expect(apiGet.mock.calls[1][0]).toBe('/api/brain/runs?limit=20&cursor_created_at=1&cursor_id=brain-b');
  expect(screen.getByRole('button', { name: '下一页' }).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: '刷新运行' }));
  await waitFor(() => expect(apiGet).toHaveBeenCalledTimes(3));
  expect(apiGet.mock.calls[2][0]).toBe(apiGet.mock.calls[1][0]);
  fireEvent.click(screen.getByRole('button', { name: '上一页' }));
  expect(await screen.findByText('brain-a')).toBeTruthy();
  expect(apiGet.mock.calls[3][0]).toBe('/api/brain/runs?limit=20');
});
it('失败保留记录和页码，迟到响应及关闭后的请求不能覆盖页面', async () => {
  let resolve;
  apiGet.mockResolvedValueOnce({ runs: [row('brain-a', '历史计划')], next_cursor: { id: 'brain-a', created_at: 1 } })
    .mockRejectedValueOnce(new Error('读取失败'))
    .mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
  const { unmount } = render(<Harness />);
  await screen.findByText('历史计划');
  fireEvent.click(screen.getByRole('button', { name: '下一页' }));
  expect(await screen.findByText('读取失败')).toBeTruthy();
  expect(screen.getByText('brain-a')).toBeTruthy();
  expect(screen.getByText('第 1 页 · 每页 20 条')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: '重试' }));
  unmount();
  expect(apiGet.mock.calls[2][1].signal.aborted).toBe(true);
  await act(async () => resolve({ runs: [row('brain-stale')], next_cursor: null }));
  expect(screen.queryByText('brain-stale')).toBeNull();
});
