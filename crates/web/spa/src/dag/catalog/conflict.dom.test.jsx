// @vitest-environment jsdom
import '../../test/setup-dom.js';
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { CatalogConflict } from './conflict.jsx';
import { apiGet } from '../../api.js';
vi.mock('../../api.js', () => ({ apiGet: vi.fn() }));
beforeEach(() => vi.clearAllMocks());

it('opens an existing catalog item through authenticated API without discarding a draft', async () => {
  apiGet.mockResolvedValue({ name: 'shared-tool', current: 2 });
  render(<><input aria-label="draft" defaultValue="unsaved description" /><CatalogConflict error={Object.assign(new Error('已有相同内容'), { body: { details: { name: 'shared-tool', url: '/api/dag/binaries/shared-tool' } } })} /></>);
  fireEvent.click(screen.getByRole('button', { name: '查看已有项：shared-tool' }));
  expect(await screen.findByText('已有项 · shared-tool')).toBeTruthy();
  expect(apiGet).toHaveBeenCalledWith('/api/dag/binaries/shared-tool');
  expect(screen.getByLabelText('draft').value).toBe('unsaved description');
});

it('shows reference failures and never follows an external inspection URL', () => {
  render(<CatalogConflict error={Object.assign(new Error('资源仍被引用: schedule:daily'), { body: { details: { name: 'tool', url: 'https://external.invalid/' } } })} />);
  expect(screen.getByText('资源仍被引用: schedule:daily')).toBeTruthy();
  expect(screen.queryByRole('button', { name: /查看已有项/ })).toBeNull();
  expect(apiGet).not.toHaveBeenCalled();
});
