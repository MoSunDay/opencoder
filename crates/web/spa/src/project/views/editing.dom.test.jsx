// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';

const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPatch: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../api.js', () => api);
vi.mock('../../fleet/detail.jsx', () => ({ ExecutionView: () => null }));
import { MdEditDrawer } from './mdDrawer.jsx';
import { TodoDrawer } from '../todoDrawer.jsx';

beforeEach(() => {
  Object.values(api).forEach((method) => method.mockReset());
  api.apiGet.mockResolvedValue({ assignments: [] });
  api.apiPatch.mockResolvedValue({ ok: true });
});

it('preserves a project edit buffer when the record object refreshes', () => {
  const seed = { id: 'p1', title: '项目', detail_md: '原文' };
  const props = { open: true, initial: seed, onOk: vi.fn(), onCancel: vi.fn() };
  const view = render(<MdEditDrawer {...props} />);
  fireEvent.change(screen.getByLabelText('detail_md'), { target: { value: '未保存的正文' } });
  view.rerender(<MdEditDrawer {...props} initial={{ ...seed }} />);
  expect(screen.getByLabelText('detail_md').value).toBe('未保存的正文');
});

it('saves TODO title, description, initiative and board column together', async () => {
  const overview = { goals: [], standalone_milestones: [], backlog: [{ id: 't1', title: '原任务', draft: '原说明', status: 'draft' }] };
  render(<TodoDrawer todoId="t1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.change(screen.getByLabelText('TODO 标题'), { target: { value: '新任务' } });
  fireEvent.change(screen.getByLabelText('任务说明'), { target: { value: '新说明' } });
  fireEvent.click(screen.getByRole('button', { name: '保存 TODO' }));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith('/api/project/todos/t1', {
    title: '新任务', draft: '新说明', milestone_id: null, board_status: 'backlog', capability_id: null,
  }));
});
