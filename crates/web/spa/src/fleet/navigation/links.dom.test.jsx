// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
import { executionHref, executionLink, linkedPage, withoutExecutionLink } from './links.js';
import { ExecutionsPanel } from '../executions.jsx';
import { apiGet } from '../../api.js';
vi.mock('../../api.js', () => ({ apiGet: vi.fn() }));
vi.mock('../detail.jsx', () => ({ ExecutionDetail: ({ id, onClose }) => <div>原生记录:{id}<button onClick={onClose}>关闭执行</button></div> }));
afterEach(() => { window.history.replaceState(null, '', '/'); vi.resetAllMocks(); });

it('encodes execution IDs and keeps execution links ahead of stored navigation', () => {
  const href = executionHref('任务 / #&?', 'dag');
  expect(executionLink(href)).toEqual({ id: '任务 / #&?', kind: 'dag' });
  expect(linkedPage(href)).toBe('topics');
  expect(linkedPage('?brain_run=brain-1')).toBe('brain');
  expect(linkedPage('?execution=+')).toBeNull();
  expect(executionLink('?execution=old', '#execution=current')).toEqual({ id: 'current', kind: '' });
  expect(linkedPage('', executionHref('redirected'))).toBe('topics');
  expect(withoutExecutionLink('http://localhost/?execution=old&keep=1#execution=new&note=keep')).toBe('/?keep=1#note=keep');
  expect(withoutExecutionLink('http://localhost/?execution=old#section')).toBe('/#section');
});

it.each(['agent', 'operator', 'team', 'dag', 'todos', 'brain'])('opens a linked %s record even when absent from the first list page', async (kind) => {
  window.history.replaceState(null, '', executionHref('historical-' + kind, kind));
  apiGet.mockResolvedValue({ nodes: [], executions: [] });
  render(<ExecutionsPanel onNotice={vi.fn()} />);
  expect(screen.getByText('原生记录:historical-' + kind)).toBeTruthy();
  await waitFor(() => expect(apiGet).toHaveBeenCalledWith('/api/executions?limit=50&kind=' + kind));
  fireEvent.click(screen.getByRole('button', { name: '关闭执行' }));
  expect(window.location.search + window.location.hash).toBe('');
  expect(screen.queryByText('原生记录:historical-' + kind)).toBeNull();
});

it('follows ID-only links, hash changes and browser history without relying on the list', async () => {
  window.history.replaceState(null, '', executionHref('first'));
  apiGet.mockResolvedValue({ nodes: [], executions: [] });
  render(<ExecutionsPanel onNotice={vi.fn()} />);
  expect(screen.getByText('原生记录:first')).toBeTruthy();
  window.history.pushState(null, '', executionHref('second'));
  window.dispatchEvent(new HashChangeEvent('hashchange'));
  expect(await screen.findByText('原生记录:second')).toBeTruthy();
  window.history.replaceState(null, '', '/?execution=legacy');
  window.dispatchEvent(new PopStateEvent('popstate'));
  expect(await screen.findByText('原生记录:legacy')).toBeTruthy();
  window.history.replaceState(null, '', '/');
  window.dispatchEvent(new PopStateEvent('popstate'));
  await waitFor(() => expect(screen.queryByText('原生记录:legacy')).toBeNull());
});

it('keeps execution list clicks in the address so refresh opens the same record', async () => {
  apiGet.mockResolvedValue({ nodes: [], executions: [{ id: 'listed-record', kind: 'operator' }] });
  render(<ExecutionsPanel onNotice={vi.fn()} />);
  fireEvent.click(await screen.findByRole('button', { name: 'listed-record' }));
  expect(executionLink(window.location.search, window.location.hash)?.id).toBe('listed-record');
  expect(await screen.findByText('原生记录:listed-record')).toBeTruthy();
});
