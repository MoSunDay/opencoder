// @vitest-environment jsdom
import '../../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { ExecutionsPanel } from '../executions.jsx';
import { apiGet } from '../../api.js';
import { executionHref } from './links.js';

vi.mock('../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../sse.js', () => ({ openStream: vi.fn(() => ({ abort() {} })) }));
afterEach(() => { cleanup(); window.history.replaceState(null, '', '/'); vi.clearAllMocks(); });

function message(seq, text) {
  const bytes = Buffer.from(JSON.stringify([{ type: 'text', text }]));
  return { seq, role: 'assistant', created_at: 1, encoding: 'base64', offset: 0,
    next_offset: bytes.length, total_bytes: bytes.length, eof: true, bytes_b64: bytes.toString('base64') };
}

it.each(['agent', 'operator'])('opens the actual linked %s Session and pages messages in older browsers', async (kind) => {
  const id = `historical-${kind}`;
  window.history.replaceState(null, '', executionHref(id));
  apiGet.mockImplementation(async (path) => {
    if (path === `/api/executions/${id}`) return {
      execution: { id, kind, node_id: 'original-runtime', status: 'done', created_at: 1 },
      request: { kind, input: {} }, session: { meta: { id } },
    };
    if (path === `/api/executions/${id}/messages`) return {
      chunks: [message(1, '# 历史会话\n这是该执行的上下文')], more: true, next_cursor: { seq: 2, offset: 0 },
    };
    if (path === `/api/executions/${id}/messages?seq=2`) return {
      chunks: [message(2, '后续执行细节')], more: false,
    };
    if (path.includes('events-page')) return { head_seq: 0 };
    if (path.includes('/inputs')) return { inputs: [] };
    return { nodes: [], executions: [] };
  });
  const at = Object.getOwnPropertyDescriptor(Array.prototype, 'at');
  const size = Object.getOwnPropertyDescriptor(URLSearchParams.prototype, 'size');
  Object.defineProperty(Array.prototype, 'at', { configurable: true, value: undefined });
  Object.defineProperty(URLSearchParams.prototype, 'size', { configurable: true, value: undefined });
  try {
    render(<ExecutionsPanel onNotice={vi.fn()} />);
    expect(await screen.findByText('会话消息')).toBeTruthy();
    expect(await screen.findByText(/历史会话/)).toBeTruthy();
    expect(screen.getByText('original-runtime')).toBeTruthy();
    fireEvent.click(await screen.findByRole('button', { name: '继续加载消息' }));
    expect(await screen.findByText('后续执行细节')).toBeTruthy();
    expect(apiGet).toHaveBeenCalledWith(`/api/executions/${id}/messages?seq=2`);
    expect(screen.getByText(id, { selector: '.ant-drawer-title' })).toBeTruthy();
    cleanup();
  } finally {
    Object.defineProperty(Array.prototype, 'at', at);
    if (size) Object.defineProperty(URLSearchParams.prototype, 'size', size);
    else delete URLSearchParams.prototype.size;
  }
});

it('keeps the linked execution visible and reports its original Runtime being offline', async () => {
  window.history.replaceState(null, '', executionHref('offline-session', 'operator'));
  apiGet.mockImplementation(async (path) => {
    if (path === '/api/executions/offline-session') throw Object.assign(new Error('offline'), { status: 503 });
    return { nodes: [], executions: [] };
  });
  render(<ExecutionsPanel onNotice={vi.fn()} />);
  await waitFor(() => expect(screen.getByText('所属节点当前离线，恢复连接后可读取明细和继续操作')).toBeTruthy());
  expect(screen.getByText('offline-session', { selector: '.ant-drawer-title' })).toBeTruthy();
  expect(screen.queryByText('会话消息')).toBeNull();
});
