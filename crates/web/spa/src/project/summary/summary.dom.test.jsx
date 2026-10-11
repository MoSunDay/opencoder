// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';

vi.mock('../../api.js', () => ({ apiGet: vi.fn(async () => ({ people: [] })) }));
import { CurrentSummary, summaryColumns } from './view.jsx';
import { MdEditDrawer } from '../views/mdDrawer.jsx';
import { ProjectTable } from '../views/projectTable.jsx';

beforeEach(() => localStorage.clear());

it('shows the saved summary and its own timestamp with an explicit empty state', () => {
  const view = render(<CurrentSummary record={{ current_summary: '## 关键进展\n接口联调完成\n## 剩余卡点\n等待设备', summary_updated_at: 1700000000000, updated_at: 1800000000000 }} />);
  expect(screen.getByRole('heading', { name: '关键进展' })).toBeTruthy();
  expect(screen.getByText('等待设备')).toBeTruthy();
  expect(screen.getByText(/概述更新时间：2023-/)).toBeTruthy();
  view.rerender(<CurrentSummary record={{ updated_at: 1800000000000 }} />);
  expect(screen.getByText('暂无概述')).toBeTruthy();
  expect(screen.getByText('概述更新时间：尚未更新')).toBeTruthy();
});

it('edits and clears the summary but omits unchanged text when only other fields change', async () => {
  const save = vi.fn(async () => false);
  render(<MdEditDrawer open title="编辑项目" initial={{ id: 'p', title: '项目', current_summary: '已完成联调' }} onOk={save} onCancel={vi.fn()} />);
  fireEvent.click(screen.getByRole('button', { name: '保 存' }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(save.mock.calls[0][0]).not.toHaveProperty('current_summary');
  fireEvent.change(screen.getByLabelText('current_summary'), { target: { value: '关键进展：验收通过\n剩余卡点：无' } });
  fireEvent.click(screen.getByRole('button', { name: '保 存' }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(2));
  expect(save.mock.calls[1][0].current_summary).toContain('验收通过');
  expect(screen.getByLabelText('current_summary').value).toContain('验收通过');
  fireEvent.change(screen.getByLabelText('current_summary'), { target: { value: '' } });
  fireEvent.click(screen.getByRole('button', { name: '保 存' }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(3));
  expect(save.mock.calls[2][0].current_summary).toBe('');
});

it('includes summary text and update time in project table filters', async () => {
  render(<ProjectTable label="概述表格" columns={summaryColumns} rows={[
    { id: 'a', current_summary: '核心工作：接口联调' },
    { id: 'b', current_summary: '剩余卡点：设备不足' },
  ]} />);
  fireEvent.click(screen.getByLabelText('筛选当前概述'));
  const input = await screen.findByLabelText('搜索当前概述');
  fireEvent.change(input, { target: { value: '设备不足' } });
  fireEvent.keyDown(input, { key: 'Enter', code: 'Enter' });
  await waitFor(() => expect(within(screen.getByLabelText('概述表格')).queryByText('核心工作：接口联调')).toBeNull());
  expect(within(screen.getByLabelText('概述表格')).getByText('剩余卡点：设备不足')).toBeTruthy();
});

it('does not overwrite a newly refreshed summary when saving an unchanged edit buffer', async () => {
  const save = vi.fn(async () => true);
  const props = { open: true, onOk: save, onCancel: vi.fn() };
  const view = render(<MdEditDrawer {...props} initial={{ id: 'p', title: '项目', current_summary: '旧概述' }} />);
  view.rerender(<MdEditDrawer {...props} initial={{ id: 'p', title: '项目', current_summary: '其他人刚更新的概述' }} />);
  fireEvent.click(screen.getByRole('button', { name: '保 存' }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(save.mock.calls[0][0]).not.toHaveProperty('current_summary');
});
