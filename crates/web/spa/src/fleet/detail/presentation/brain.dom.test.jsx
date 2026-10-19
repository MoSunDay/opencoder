// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { apiGet } from '../../../api.js';
import { ExecutionView } from '../../detail.jsx';
vi.mock('../../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn() }));
vi.mock('../../../sse.js', () => ({ openStream: () => ({ abort() {} }) }));
vi.mock('../liveTranscript.js', () => ({ useExecutionTranscript: () => ({ caughtUp: false }) }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
const detail = (id, text) => ({ execution: { id, kind: 'operator', status: 'done', created_at: 1 }, request: { kind: 'operator', input: { task: '输入内容' } }, result: { scheduler_output: { summary: text, passed: false } } });
it('默认显示结果，过程及实际输入按页签查看，单一明细读取并保留托管操作限制', async () => {
  apiGet.mockImplementation(async (path) => path.endsWith('/operator-a') ? detail('operator-a', '执行结论') : { messages: [] });
  render(<ExecutionView executionRef={{ id: 'operator-a', kind: 'operator' }} presentation="brain" managed />);
  expect(await screen.findByText('执行结论')).toBeTruthy();
  expect(screen.getByText('false')).toBeTruthy();
  expect(screen.queryByText('会话消息')).toBeNull();
  fireEvent.click(screen.getByRole('tab', { name: '执行过程' }));
  expect(screen.getByText('会话消息')).toBeTruthy();
  fireEvent.click(screen.getByRole('tab', { name: '输入与产物' }));
  expect(screen.getByText('输入内容')).toBeTruthy();
  expect(apiGet.mock.calls.filter(([path]) => path === '/api/executions/operator-a')).toHaveLength(1);
  expect(screen.queryByRole('button', { name: '取消（终止）' })).toBeNull();
});
it('切换执行和关闭时取消读取，迟到结果不能覆盖当前执行', async () => {
  let finishA;
  apiGet.mockImplementation((path) => path.endsWith('/operator-a') ? new Promise((resolve) => { finishA = resolve; })
    : Promise.resolve(path.endsWith('/operator-b') ? detail('operator-b', '新结果') : { messages: [] }));
  const { rerender, unmount } = render(<ExecutionView executionRef={{ id: 'operator-a', kind: 'operator' }} presentation="brain" managed />);
  const signal = apiGet.mock.calls[0][1].signal;
  rerender(<ExecutionView executionRef={{ id: 'operator-b', kind: 'operator' }} presentation="brain" managed />);
  expect(await screen.findByText('新结果')).toBeTruthy();
  expect(signal.aborted).toBe(true);
  await act(async () => finishA(detail('operator-a', '迟到结果')));
  expect(screen.queryByText('迟到结果')).toBeNull();
  unmount();
  expect(apiGet.mock.calls.find(([path]) => path.endsWith('/operator-b'))[1].signal.aborted).toBe(true);
});
it('节点离线明确报错并可以重试', async () => {
  apiGet.mockRejectedValueOnce(Object.assign(new Error('offline'), { status: 503 }));
  render(<ExecutionView executionRef={{ id: 'operator-a', kind: 'operator' }} presentation="brain" managed />);
  expect(await screen.findByText('所属节点当前离线，恢复连接后可读取明细和继续操作')).toBeTruthy();
  apiGet.mockImplementation(async (path) => path.endsWith('/operator-a') ? detail('operator-a', '恢复结果') : { messages: [] });
  fireEvent.click(screen.getByRole('button', { name: '重试' }));
  await waitFor(() => expect(screen.getByText('恢复结果')).toBeTruthy());
});
